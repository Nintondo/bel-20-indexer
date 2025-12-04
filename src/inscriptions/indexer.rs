use hashbrown::HashMap;
use std::time::Instant;

use super::*;
use crate::inscriptions::utils::{PrevoutCache, PREVOUT_CACHE_CAPACITY};
use crate::tokens::BlockTokenState;
use crate::utils::timing::INDEXING_METRICS;

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
        let handle_start = Instant::now();
        INDEXING_METRICS.note_task_start(handle_start);

        // Decide whether we're in the pre-FIB fast-sync region
        let fib = self.server.indexer.coin.fib.unwrap_or_default();
        let is_pre_fib = block_height < fib;

        // If we're switching to immediate-commit mode (post-FIB or near tip), flush any pending pre-FIB batch first
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

            // Build processed data for this block, but don't commit to DB yet
            let mut to_write = DataToWrite::default();
            self.handle_block(&mut to_write, block_height, block, false)?;
            self.pending_blocks += 1;
            self.pending_processed.extend(to_write.processed);

            // Record timing for pre-FIB blocks
            INDEXING_METRICS.record_block_handle(handle_start.elapsed());
            INDEXING_METRICS.note_task_end(Instant::now());
            INDEXING_METRICS.maybe_print(5);

            // No events/history pre-FIB
            return Ok(());
        }

        // Immediate commit path (post-FIB or near tip with reorg protection)
        let mut to_write = DataToWrite::default();
        self.handle_block(&mut to_write, block_height, block, handle_reorgs)?;

        if handle_reorgs {
            self.reorg_cache.lock().new_block(block_height);
        }

        let db_write_start = Instant::now();
        let mut db_batch = DbBatch::new(&self.server.db);

        // write/remove data from block
        for data in &to_write.processed {
            data.write(&self.server, handle_reorgs.then_some(self.reorg_cache.clone()), &mut db_batch);
        }

        // Commit DB changes before emitting events, preserving original ordering semantics.
        db_batch.write();
        INDEXING_METRICS.record_db_write(db_write_start.elapsed());

        let event_emit_start = Instant::now();
        for event in to_write.block_events {
            self.server.event_sender.send(event).ok();
        }
        INDEXING_METRICS.record_event_emit(event_emit_start.elapsed());

        let history_start = Instant::now();
        if self.server.raw_event_sender.send(to_write.history).is_err() && !self.server.token.is_cancelled() {
            panic!("Failed to send raw event");
        }
        INDEXING_METRICS.record_history_send(history_start.elapsed());

        // Record total block handle time
        INDEXING_METRICS.record_block_handle(handle_start.elapsed());
        INDEXING_METRICS.note_task_end(Instant::now());
        INDEXING_METRICS.maybe_print(5);

        Ok(())
    }

    pub fn finalize(&mut self) -> anyhow::Result<()> {
        self.flush_pending_batch()
    }

    fn flush_pending_batch(&mut self) -> anyhow::Result<()> {
        if self.pending_processed.is_empty() {
            return Ok(());
        }

        let flush_start = Instant::now();
        let db_write_start = Instant::now();
        let mut db_batch = DbBatch::new(&self.server.db);
        for data in &self.pending_processed {
            // No reorg journal for deep sync; pass None
            data.write(&self.server, None, &mut db_batch);
        }
        db_batch.write();
        INDEXING_METRICS.record_db_write(db_write_start.elapsed());
        INDEXING_METRICS.record_prefib_flush(flush_start.elapsed());

        self.pending_processed.clear();
        self.pending_blocks = 0;
        // After a successful pre-FIB batch flush, drop the in-memory prevout cache
        // so that batching can resume instead of staying near capacity.
        // self.prevout_cache.clear();
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

        let prevout_start = Instant::now();
        let prevouts = utils::process_prevouts(self.server.db.clone(), &block, &mut to_write.processed, &mut self.prevout_cache)?;
        INDEXING_METRICS.record_prevout_process(prevout_start.elapsed());

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

        let token_state_load_start = Instant::now();
        let mut runtime_guard = self.server.token_state.lock();
        let mut block_token_state = BlockTokenState::new(&mut *runtime_guard, self.server.clone(), prevouts);
        INDEXING_METRICS.record_token_cache_load(token_state_load_start.elapsed());

        {
            let mut parser = Parser { server: &self.server };

            // Build a per-block map of txid -> tx so we can resolve script addresses for recipients
            let parse_start = Instant::now();
            parser.parse_block(block_height, &block, prevouts, &mut to_write.processed, &mut block_token_state);
            INDEXING_METRICS.record_block_parse(parse_start.elapsed());
        }

        // Build a per-block map of txid -> tx so we can resolve script addresses for recipients
        let tx_by_id: HashMap<Txid, _> = block.txs.iter().map(|tx| (tx.hash.into(), tx)).collect();

        let mut fullhash_to_load = HashSet::new();

        let token_process_start = Instant::now();
        let (history_actions, tokens_pd) = block_token_state.finish(&self.server.holders, block_height, block.header.value.timestamp);
        to_write.history = history_actions
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
        INDEXING_METRICS.record_token_cache_process(token_process_start.elapsed());

        // Resolve address strings for hashes touched in this block.
        //
        // IMPORTANT: the `outpoint` stored in TokenHistoryDB always points to the
        // *output that carries the inscription / transfer*. For transfer sends this
        // is the **recipient** output, while the corresponding sender history row
        // uses the same outpoint but a different address hash. If we bound that
        // sender hash to the recipient script we would later see the same textual
        // address at multiple ranks in the holders view. To avoid that, we only
        // derive address strings for history variants where the outpoint's script
        // actually belongs to `addr_token.address`.
        let mut new_block_addresses = HashMap::<FullHash, String>::new();
        for (addr_token, history_value) in &to_write.history {
            let addr = addr_token.address;
            if addr == *OP_RETURN_HASH || new_block_addresses.contains_key(&addr) {
                continue;
            }

            match &history_value.action {
                // For Send rows the outpoint belongs to the *recipient* output,
                // not to the sender hash stored in `addr_token.address`, so skip.
                TokenHistoryDB::Send { .. } => {}
                // For all other variants the outpoint's script corresponds to
                // the hash carried in `addr_token.address`.
                TokenHistoryDB::Deploy { .. }
                | TokenHistoryDB::Mint { .. }
                | TokenHistoryDB::DeployTransfer { .. }
                | TokenHistoryDB::Receive { .. }
                | TokenHistoryDB::SendReceive { .. } => {
                    let outpoint = history_value.action.outpoint();
                    if let Some(tx) = tx_by_id.get(&outpoint.txid) {
                        if let Some(output) = tx.value.outputs.get(outpoint.vout as usize) {
                            let script = bellscoin::Script::from_bytes(&output.out.script_pubkey);
                            if let Some(address) =
                                nint_blk::proto::script::script_to_address_str(&script, self.server.indexer.coin)
                            {
                                new_block_addresses.entry(addr).or_insert_with(|| address);
                            }
                        }
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

        let rest_addresses: AddressesFullHash =
            std::collections::HashMap::from_iter(combined_addresses.into_iter()).into();

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

        to_write.processed.push(tokens_pd);

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
