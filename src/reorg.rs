use super::*;

pub const REORG_CACHE_MAX_LEN: usize = 30;

pub enum TokenHistoryEntry {
    BalancesBefore(Vec<(AddressToken, TokenBalance)>),
    BalancesToRemove(Vec<AddressToken>),
    DeploysToRemove(Vec<LowerCaseTokenTick>),
    RestoreTransfers(Vec<(AddressLocation, TransferProtoDB)>),
    RemoveTransfers(Vec<AddressLocation>),
    RemoveHistory {
        to_remove: Vec<AddressTokenIdDB>,
        last_history_id: u64,
        outpoint_to_event: Vec<OutPoint>,
        height: u32,
    },
}

trait ProceedReorg: Sized {
    fn proceed(self, server: &Server) -> anyhow::Result<()>;
}

impl ProceedReorg for TokenHistoryEntry {
    fn proceed(self, server: &Server) -> anyhow::Result<()> {
        match self {
            TokenHistoryEntry::DeploysToRemove(to_remove) => {
                server.db.token_to_meta.remove_batch(to_remove);
            }
            TokenHistoryEntry::BalancesBefore(items) => {
                server.db.address_token_to_balance.extend(items);
            }
            TokenHistoryEntry::BalancesToRemove(address_tokens) => {
                server.db.address_token_to_balance.remove_batch(address_tokens);
            }
            TokenHistoryEntry::RestoreTransfers(items) => {
                server.db.address_location_to_transfer.extend(items);
            }
            TokenHistoryEntry::RemoveTransfers(address_locations) => {
                server.db.address_location_to_transfer.remove_batch(address_locations);
            }
            TokenHistoryEntry::RemoveHistory {
                to_remove,
                last_history_id,
                outpoint_to_event,
                height,
            } => {
                server.db.last_history_id.set((), last_history_id);
                server.db.block_events.remove(height);
                server.db.address_token_to_history.remove_batch(to_remove);
                server.db.outpoint_to_event.remove_batch(outpoint_to_event);
            }
        }

        Ok(())
    }
}

pub enum OrdinalsEntry {
    RestoreOffsets(Vec<(OutPoint, HashSet<u64>)>),
    RemoveOffsets(Vec<OutPoint>),
    RestorePrevouts(Vec<(OutPoint, TxOut)>),
    RestorePartial(Vec<(OutPoint, Partials)>),
    RemovePartials(Vec<OutPoint>),
}

impl ProceedReorg for OrdinalsEntry {
    fn proceed(self, server: &Server) -> anyhow::Result<()> {
        match self {
            OrdinalsEntry::RestoreOffsets(items) => {
                server.db.outpoint_to_inscription_offsets.extend(items);
            }
            OrdinalsEntry::RemoveOffsets(outpoints) => {
                server.db.outpoint_to_inscription_offsets.remove_batch(outpoints);
            }
            OrdinalsEntry::RestorePrevouts(items) => {
                server.db.prevouts.extend(items);
            }
            OrdinalsEntry::RestorePartial(items) => {
                server.db.outpoint_to_partials.extend(items);
            }
            OrdinalsEntry::RemovePartials(outpoints) => {
                server.db.outpoint_to_partials.remove_batch(outpoints);
            }
        }

        Ok(())
    }
}

#[derive(Default)]
struct ReorgHistoryBlock {
    token_history: Vec<TokenHistoryEntry>,
    ordinals_history: Vec<OrdinalsEntry>,
}

impl ReorgHistoryBlock {
    fn new() -> Self {
        Self::default()
    }
}

pub struct ReorgCache {
    blocks: BTreeMap<u32, ReorgHistoryBlock>,
    len: usize,
}

impl ReorgCache {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            len: REORG_CACHE_MAX_LEN,
        }
    }

    pub fn new_block(&mut self, block_height: u32) {
        if self.blocks.len() == self.len {
            self.blocks.pop_first();
        }
        self.blocks.insert(block_height, ReorgHistoryBlock::new());
    }

    pub fn push_ordinals_entry(&mut self, data: OrdinalsEntry) {
        self.blocks.last_entry().unwrap().get_mut().ordinals_history.push(data);
    }

    pub fn push_token_entry(&mut self, data: TokenHistoryEntry) {
        self.blocks.last_entry().unwrap().get_mut().token_history.push(data);
    }

    pub fn restore(&mut self, server: &Server, block_height: u32) -> anyhow::Result<()> {
        while !self.blocks.is_empty() && block_height <= *self.blocks.last_key_value().unwrap().0 {
            let (height, data) = self.blocks.pop_last().anyhow()?;

            server.db.last_block.set((), height - 1);
            server.db.block_info.remove(height);

            for entry in data.token_history.into_iter().rev() {
                entry.proceed(server)?;
            }
            for entry in data.ordinals_history.into_iter().rev() {
                entry.proceed(server)?;
            }
        }

        Ok(())
    }

    pub fn restore_all(&mut self, server: &Server) -> anyhow::Result<()> {
        let from = self.blocks.first_key_value().map(|x| *x.0);
        let to = self.blocks.last_key_value().map(|x| *x.0);

        warn!("Restoring savepoints from {:?} to {:?}", from, to);
        self.restore(server, 0)
    }
}
