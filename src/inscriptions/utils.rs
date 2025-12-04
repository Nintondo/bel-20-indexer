use bellscoin::{Script, hashes::{sha256, Hash}};
use nint_blk::proto::hashbrown::HashMap;
use nint_blk::proto::block::Block;
use rayon::prelude::*;
use std::collections::VecDeque;

use super::{process_data::ProcessedData, *};

pub const PREVOUT_CACHE_CAPACITY: usize = 10_000_000;

pub struct PrevoutCache {
    map: HashMap<OutPoint, TxPrevout>,
    order: VecDeque<OutPoint>,
    capacity: usize,
    result_scratch: HashMap<OutPoint, TxPrevout>,
}

impl PrevoutCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity,
            result_scratch: HashMap::new(),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.result_scratch.clear();
    }

    #[inline]
    pub fn get(&self, key: &OutPoint) -> Option<TxPrevout> {
        self.map.get(key).copied()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn insert(&mut self, key: OutPoint, value: TxPrevout) {
        // insert returns Some(old) if key existed, None otherwise.
        if self.map.insert(key, value).is_none() {
            self.order.push_back(key);
            if self.map.len() > self.capacity {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }

    pub fn insert_block_outputs<I>(&mut self, outputs: I)
    where
        I: IntoIterator<Item = (OutPoint, TxPrevout)>,
    {
        for (k, v) in outputs {
            self.insert(k, v);
        }
    }
}

pub fn process_prevouts<'a>(
    db: Arc<DB>,
    block: &Block,
    data_to_write: &mut Vec<ProcessedData>,
    cache: &'a mut PrevoutCache,
) -> anyhow::Result<&'a HashMap<OutPoint, TxPrevout>> {
    let outputs_capacity: usize = block.txs.iter().map(|tx| tx.value.outputs.len()).sum();
    let outputs_start = std::time::Instant::now();
    let mut prevouts: HashMap<OutPoint, TxPrevout> = HashMap::with_capacity(outputs_capacity);

    for tx in &block.txs {
        let txid = tx.hash;
        for (vout, txout) in tx.value.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid: txid.into(),
                vout: vout as u32,
            };

            let script_bytes = &txout.out.script_pubkey;
            let script = Script::from_bytes(script_bytes);
            if script.is_provably_unspendable() {
                continue;
            }

            let script_hash = sha256::Hash::hash(script_bytes).into();
            prevouts.insert(outpoint, TxPrevout { script_hash, value: txout.out.value });
        }
    }
    INDEXING_METRICS.record_prevout_build(outputs_start.elapsed());

    let inputs_start = std::time::Instant::now();
    let txids_keys = block
        .txs
        .iter()
        .filter(|tx| !tx.value.is_coinbase())
        .flat_map(|tx| tx.value.inputs.iter().map(|x| x.outpoint))
        .unique()
        .collect_vec();
    INDEXING_METRICS.record_prevout_inputs(inputs_start.elapsed());

    cache.result_scratch.clear();
    cache.result_scratch.reserve(txids_keys.len());

    if !txids_keys.is_empty() {
        let mut missing_keys = Vec::new();
        let cache_lookup_start = std::time::Instant::now();

        // Try cache first; if not found, check outputs from this block
        // before hitting RocksDB. This skips DB work for in-block spends.
        for key in &txids_keys {
            if let Some(val) = cache.get(key) {
                cache.result_scratch.insert(*key, val);
            } else if let Some(local) = prevouts.get(key) {
                cache.insert(*key, *local);
                cache.result_scratch.insert(*key, *local);
            } else {
                missing_keys.push(*key);
            }
        }
        INDEXING_METRICS.record_prevout_cache_lookup(cache_lookup_start.elapsed());

        if !missing_keys.is_empty() {
            missing_keys.sort_unstable();
            let db_fetch_start = std::time::Instant::now();
            const CHUNK: usize = 512;
            let table = &db.prevouts;
            let chunked: Vec<Vec<OutPoint>> = missing_keys.chunks(CHUNK).map(|c| c.to_vec()).collect();
            let chunk_results: Vec<Vec<(OutPoint, Option<TxPrevout>)>> = chunked
                .into_par_iter()
                .map(|chunk| {
                    let from_db = table.multi_get(chunk.iter());
                    chunk.into_iter().zip(from_db).collect()
                })
                .collect();
            let db_fetch_elapsed = db_fetch_start.elapsed();

            for (key, maybe_val) in chunk_results.into_iter().flatten() {
                match maybe_val {
                    Some(val) => {
                        cache.insert(key, val);
                        cache.result_scratch.insert(key, val);
                    }
                    None => {
                        if let Some(value) = prevouts.get(&key) {
                            cache.insert(key, *value);
                            cache.result_scratch.insert(key, *value);
                        } else {
                            return Err(anyhow::anyhow!("Missing prevout for key {}. Block: {}", key, block.header.hash));
                        }
                    }
                }
            }
            INDEXING_METRICS.record_prevout_db_fetch(db_fetch_elapsed);
        }
    }

    let cache_insert_start = std::time::Instant::now();
    cache.insert_block_outputs(prevouts.iter().map(|(k, v)| (*k, *v)));
    INDEXING_METRICS.record_prevout_cache_insert(cache_insert_start.elapsed());

    data_to_write.push(ProcessedData::Prevouts {
        to_write: prevouts,
        to_remove: txids_keys,
    });

    Ok(&cache.result_scratch)
}
