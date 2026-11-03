use super::*;

pub const REORG_CACHE_MAX_LEN: usize = 30;

pub enum TokenHistoryEntry {
    BalancesBefore(Vec<(AddressToken, TokenBalance)>),
    BalancesToRemove(Vec<AddressToken>),
    DeploysToRemove(Vec<LowerCaseTokenTick>),
    DeploysToRestore(Vec<(LowerCaseTokenTick, TokenMetaDB)>),
    RestoreTransfers(Vec<(AddressLocation, TransferProtoDB)>),
    RemoveTransfers(Vec<AddressLocation>),
    RemoveHistory {
        to_remove: Vec<AddressTokenIdDB>,
        last_history_id: u64,
        outpoint_to_event: Vec<OutPoint>,
        height: u32,
        token_id_to_event: Vec<TokenId>,
    },
}

trait ProceedReorg: Sized {
    fn proceed(self, server: &Server, batch: &mut rocksdb_wrapper::RocksBatch) -> anyhow::Result<()>;
}

impl ProceedReorg for TokenHistoryEntry {
    fn proceed(self, server: &Server, batch: &mut rocksdb_wrapper::RocksBatch) -> anyhow::Result<()> {
        match self {
            TokenHistoryEntry::DeploysToRemove(to_remove) => {
                batch.remove_batch(&server.db.token_to_meta, to_remove);
            }
            TokenHistoryEntry::DeploysToRestore(items) => {
                batch.extend(&server.db.token_to_meta, items);
            }
            TokenHistoryEntry::BalancesBefore(items) => {
                batch.extend(&server.db.address_token_to_balance, items);
            }
            TokenHistoryEntry::BalancesToRemove(address_tokens) => {
                batch.remove_batch(&server.db.address_token_to_balance, address_tokens);
            }
            TokenHistoryEntry::RestoreTransfers(items) => {
                batch.extend(&server.db.address_location_to_transfer, items);
            }
            TokenHistoryEntry::RemoveTransfers(address_locations) => {
                batch.remove_batch(&server.db.address_location_to_transfer, address_locations);
            }
            TokenHistoryEntry::RemoveHistory {
                to_remove,
                last_history_id,
                outpoint_to_event,
                height,
                token_id_to_event,
            } => {
                batch.set(&server.db.last_history_id, (), last_history_id);
                batch.remove(&server.db.block_events, height);
                batch.remove_batch(&server.db.address_token_to_history, to_remove);
                batch.remove_batch(&server.db.outpoint_to_event, outpoint_to_event);
                batch.remove_batch(&server.db.token_id_to_event, token_id_to_event);
            }
        }

        Ok(())
    }
}

pub enum OrdinalsEntry {
    RestoreOffsets(Vec<(OutPoint, HashSet<u64>)>),
    RemoveOffsets(Vec<OutPoint>),
    RestorePrevouts(Vec<(OutPoint, TxPrevout)>),
    RestorePartial(Vec<(OutPoint, Partials)>),
    RemovePartials(Vec<OutPoint>),
}

impl ProceedReorg for OrdinalsEntry {
    fn proceed(self, server: &Server, batch: &mut rocksdb_wrapper::RocksBatch) -> anyhow::Result<()> {
        match self {
            OrdinalsEntry::RestoreOffsets(items) => {
                batch.extend(&server.db.outpoint_to_inscription_offsets, items);
            }
            OrdinalsEntry::RemoveOffsets(outpoints) => {
                batch.remove_batch(&server.db.outpoint_to_inscription_offsets, outpoints);
            }
            OrdinalsEntry::RestorePrevouts(items) => {
                batch.extend(&server.db.prevouts, items);
            }
            OrdinalsEntry::RestorePartial(items) => {
                batch.extend(&server.db.outpoint_to_partials, items);
            }
            OrdinalsEntry::RemovePartials(outpoints) => {
                batch.remove_batch(&server.db.outpoint_to_partials, outpoints);
            }
        }

        Ok(())
    }
}

#[derive(Default)]
pub struct ReorgHistoryBlock {
    token_history: Vec<TokenHistoryEntry>,
    ordinals_history: Vec<OrdinalsEntry>,
}

impl ReorgHistoryBlock {
    fn new() -> Self {
        Self::default()
    }
}

pub struct ReorgCache {
    pub blocks: BTreeMap<u32, ReorgHistoryBlock>,
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
        let _gate = server.sync_gate.write();
        while !self.blocks.is_empty() && block_height < *self.blocks.last_key_value().unwrap().0 {
            let (height, data) = self.blocks.pop_last().anyhow()?;
            let mut batch = rocksdb_wrapper::RocksBatch::default();
            batch.remove(&server.db.sync_blocks, height);
            batch.remove(&server.db.proof_of_history, height);

            batch.set(&server.db.last_block, (), height - 1);
            batch.remove(&server.db.block_info, height);

            for entry in data.token_history.into_iter().rev() {
                entry.proceed(server, &mut batch)?;
            }
            for entry in data.ordinals_history.into_iter().rev() {
                entry.proceed(server, &mut batch)?;
            }
            batch.commit(&server.db.last_block.db)?;
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
