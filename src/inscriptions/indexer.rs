use std::collections::HashMap;

use super::*;
use crate::inscriptions::utils::{PrevoutCache, PREVOUT_CACHE_CAPACITY};

// Pre-FIB batching controls
const PRE_FIB_BATCH_MAX_BLOCKS: usize = 1_000; // flush at most every N pre-FIB blocks
const PREVOUT_CACHE_SAFETY_MARGIN: usize = 200_000; // keep headroom to avoid evicting uncommitted prevouts

pub struct InscriptionIndexer {
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<ReorgCache>>,
    prevout_cache: PrevoutCache,
    // Pre-FIB batching
    pending_processed: Vec<ProcessedData>,
    pending_blocks: usize,
}

#[derive(Default)]
pub struct DataToWrite {
    pub processed: Vec<ProcessedData>,
    pub block_events: Vec<ServerEvent>,
    pub history: Vec<(AddressTokenIdDB, HistoryValue)>,
}

impl InscriptionIndexer {
    pub fn new(server: Arc<Server>, reorg_cache: Arc<parking_lot::Mutex<ReorgCache>>) -> Self {
        Self {
            reorg_cache,
            server,
            prevout_cache: PrevoutCache::new(PREVOUT_CACHE_CAPACITY),
            pending_processed: Vec::new(),
            pending_blocks: 0,
        }
    }
    pub fn handle(&mut self, block_height: u32, block: nint_blk::proto::block::Block, handle_reorgs: bool) -> anyhow::Result<()> {
        // Decide whether we’re in the pre-FIB fast-sync region
        let fib = self.server.indexer.coin.fib.unwrap_or_default();
        let is_pre_fib = block_height < fib;

        // If we’re switching to immediate-commit mode (post-FIB or near tip), flush any pending pre-FIB batch first
        if (!is_pre_fib || handle_reorgs) && !self.pending_processed.is_empty() {
            self.flush_pending_batch()?;
        }

        // Pre-FIB batching path when far from tip (no reorg handling applied)
        if is_pre_fib && !handle_reorgs {
            // Predict cache growth to avoid evicting uncommitted outputs:
            // - outputs are always inserted into cache
            // - inputs missing from cache are inserted when loaded from DB
            let predicted_outputs: usize = block.txs.iter().map(|tx| tx.value.outputs.len()).sum();
            let predicted_missing_inputs: usize = block
                .txs
                .iter()
                .filter(|tx| !tx.value.is_coinbase())
                .flat_map(|tx| tx.value.inputs.iter())
                .filter(|inp| self.prevout_cache.get(&inp.outpoint).is_none())
                .count();
            let predicted_additions = predicted_outputs + predicted_missing_inputs;

            let cache_cap = self.prevout_cache.capacity();
            let cache_len = self.prevout_cache.len();
            let need_flush_for_cache = cache_len + predicted_additions + PREVOUT_CACHE_SAFETY_MARGIN >= cache_cap;
            let need_flush_for_blocks = self.pending_blocks >= PRE_FIB_BATCH_MAX_BLOCKS;

            if (need_flush_for_cache || need_flush_for_blocks) && !self.pending_processed.is_empty() {
                self.flush_pending_batch()?;
            }

            // Build processed data for this block, but don’t commit to DB yet
            let mut to_write = DataToWrite::default();
            self.handle_block(&mut to_write, block_height, block, false)?;
            self.pending_blocks += 1;
            self.pending_processed.extend(to_write.processed);
            // No events/history pre-FIB
            return Ok(());
        }

        // Immediate commit path (post-FIB or near tip with reorg protection)
        let mut to_write = DataToWrite::default();
        self.handle_block(&mut to_write, block_height, block, handle_reorgs)?;

        if handle_reorgs {
            self.reorg_cache.lock().new_block(block_height);
        }

        // Snapshot token deltas (if any) before we consume processed data
        let tokens_delta = to_write.processed.iter().find_map(|pd| {
            if let ProcessedData::Tokens {
                metas,
                balances,
                transfers_to_write,
                transfers_to_remove,
            } = pd
            {
                Some((metas, balances, transfers_to_write, transfers_to_remove))
            } else {
                None
            }
        });

        let mut db_batch = DbBatch::new(&self.server.db);

        // write/remove data from block
        for data in &to_write.processed {
            data.write(&self.server, handle_reorgs.then_some(self.reorg_cache.clone()), &mut db_batch);
        }

        // Commit DB changes before emitting events, preserving original ordering semantics.
        db_batch.write();

        if let Some((metas, balances, transfers_to_write, transfers_to_remove)) = tokens_delta {
            let mut rt = self.server.token_state.lock();
            rt.apply_tokens_delta(metas, balances, transfers_to_write, transfers_to_remove);
        }

        for event in to_write.block_events {
            self.server.event_sender.send(event).ok();
        }

        if self.server.raw_event_sender.send(to_write.history).is_err() && !self.server.token.is_cancelled() {
            panic!("Failed to send raw event");
        }

        Ok(())
    }

    pub fn finalize(&mut self) -> anyhow::Result<()> {
        self.flush_pending_batch()
    }

    fn flush_pending_batch(&mut self) -> anyhow::Result<()> {
        if self.pending_processed.is_empty() {
            return Ok(());
        }

        let mut db_batch = DbBatch::new(&self.server.db);
        for data in &self.pending_processed {
            // No reorg journal for deep sync; pass None
            data.write(&self.server, None, &mut db_batch);
        }
        db_batch.write();

        self.pending_processed.clear();
        self.pending_blocks = 0;
        Ok(())
    }

    fn handle_block(&mut self, to_write: &mut DataToWrite, block_height: u32, block: nint_blk::proto::block::Block, handle_reorgs: bool) -> anyhow::Result<()> {
        let current_hash = block.header.hash;

        let mut last_history_id = self.server.db.last_history_id.get(()).unwrap_or_default();

        if handle_reorgs {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let block_info = BlockInfo {
            created: block.header.value.timestamp,
            hash: current_hash.into(),
        };

        let prev_block_height = block_height.checked_sub(1).unwrap_or_default();
        let prev_block_proof = self.server.db.proof_of_history.get(prev_block_height).unwrap_or(*DEFAULT_HASH);

        let prevouts = utils::process_prevouts(self.server.db.clone(), &block, &mut to_write.processed, Some(&mut self.prevout_cache))?;

        if block_height < self.server.indexer.coin.fib.unwrap_or_default() {
            to_write.processed.push(ProcessedData::BlockWithoutProof {
                block_number: block_height,
                block_info,
            });

            return Ok(());
        }

        if block.txs.len() == 1 {
            let new_proof = Server::generate_history_hash(prev_block_proof, &[], &Default::default())?;

            to_write.processed.push(ProcessedData::Info {
                block_number: block_height,
                block_info,
                block_proof: new_proof,
            });

            to_write.block_events.push(ServerEvent::NewBlock(block_height, new_proof, block.header.hash.into()));

            return Ok(());
        }

        let runtime_guard = self.server.token_state.lock();
        let mut token_cache = TokenCache::load(&prevouts, self.server.clone(), &runtime_guard);

        let transfers_to_remove = token_cache
            .valid_transfers
            .iter()
            .map(|(key, value)| AddressLocation { address: value.0, location: *key })
            .collect::<HashSet<_>>();

        let mut parser = Parser {
            token_cache: &mut token_cache,
            server: &self.server,
        };

        // Build a per-block map of txid -> tx so we can resolve script addresses for recipients
        let tx_by_id: HashMap<Txid, _> = block.txs.iter().map(|tx| (tx.hash.into(), tx)).collect();

        parser.parse_block(block_height, &block, &prevouts, &mut to_write.processed);

        token_cache.load_tokens_data(&self.server.db, &runtime_guard)?;

        let mut fullhash_to_load = HashSet::new();

        to_write.history = token_cache
            .process_token_actions(&self.server.holders)
            .into_iter()
            .flat_map(|action| {
                last_history_id += 1;
                let mut results: Vec<(AddressTokenIdDB, HistoryValue)> = vec![];
                let token = action.tick();
                let recipient = action.recipient();
                fullhash_to_load.insert(recipient);
                let key = AddressTokenIdDB {
                    address: recipient,
                    token,
                    id: last_history_id,
                };
                let db_action = TokenHistoryDB::from_token_history(action.clone());
                if let TokenHistoryDB::Send { amt, txid, vout, .. } = db_action {
                    let sender = action.sender().unwrap();
                    fullhash_to_load.insert(sender);
                    last_history_id += 1;
                    results.extend([
                        (
                            AddressTokenIdDB {
                                address: sender,
                                token,
                                id: last_history_id,
                            },
                            HistoryValue {
                                height: block_height,
                                action: db_action,
                            },
                        ),
                        (
                            key,
                            HistoryValue {
                                height: block_height,
                                action: TokenHistoryDB::Receive { amt, sender, txid, vout },
                            },
                        ),
                    ])
                } else {
                    results.push((
                        key,
                        HistoryValue {
                            action: db_action,
                            height: block_height,
                        },
                    ));
                }

                results
            })
            .collect();

        // Resolve address strings for recipients in this block.
        let mut new_block_addresses = HashMap::<FullHash, String>::new();
        for (addr_token, history_value) in &to_write.history {
            let addr = addr_token.address;
            if addr == *OP_RETURN_HASH || new_block_addresses.contains_key(&addr) {
                continue;
            }

            let outpoint = history_value.action.outpoint();
            if let Some(tx) = tx_by_id.get(&outpoint.txid) {
                if let Some(output) = tx.value.outputs.get(outpoint.vout as usize) {
                    if let Some(address) = output.script.address.as_ref() {
                        new_block_addresses.entry(addr).or_insert_with(|| address.to_owned());
                    }
                }
            }
        }

        // Load already-known mappings for any BRC addresses we couldn't resolve from this block.
        let hashes_to_load: Vec<FullHash> = fullhash_to_load
            .iter()
            .filter(|hash| !new_block_addresses.contains_key(*hash) && **hash != *OP_RETURN_HASH)
            .copied()
            .collect();

        let existing: HashMap<FullHash, String> = if hashes_to_load.is_empty() {
            HashMap::new()
        } else {
            self.server
                .db
                .fullhash_to_address
                .multi_get_kv(hashes_to_load.iter(), false)
                .into_iter()
                .map(|(k, v)| (*k, v))
                .collect()
        };

        let mut combined_addresses = existing;
        for (hash, address) in &new_block_addresses {
            combined_addresses.entry(*hash).or_insert_with(|| address.clone());
        }

        let rest_addresses: AddressesFullHash = combined_addresses.into();

        let new_proof = Server::generate_history_hash(prev_block_proof, &to_write.history, &rest_addresses)?;

        if !new_block_addresses.is_empty() {
            to_write.processed.push(ProcessedData::FullHash {
                addresses: new_block_addresses.into_iter().collect(),
            });
        }

        to_write.processed.push(ProcessedData::History {
            block_number: block_height,
            last_history_id,
            history: to_write.history.clone(),
        });

        to_write.processed.push(ProcessedData::Tokens {
            metas: token_cache.tokens.into_iter().map(|(k, v)| (k, TokenMetaDB::from(v))).collect(),
            balances: token_cache.token_accounts.into_iter().collect(),
            transfers_to_write: token_cache
                .valid_transfers
                .into_iter()
                .map(|(location, (address, proto))| (AddressLocation { address, location }, proto))
                .collect(),
            transfers_to_remove: transfers_to_remove.into_iter().collect(),
        });

        to_write.block_events.push(ServerEvent::NewBlock(block_height, new_proof, current_hash.into()));

        to_write.processed.push(ProcessedData::Info {
            block_number: block_height,
            block_info,
            block_proof: new_proof,
        });

        Ok(())
    }
}

#[derive(Debug)]
pub enum ParsedInscriptionResult {
    None,
    Partials,
    Single(Box<InscriptionTemplate>),
    Many(Vec<InscriptionTemplate>),
}
