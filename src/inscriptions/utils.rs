use bellscoin::ScriptBuf;
use nint_blk::proto::block::Block;

use super::{process_data::ProcessedData, *};

pub const PREVOUT_CACHE_CAPACITY: usize = 10_000_000;

pub struct PrevoutCache {
    map: HashMap<OutPoint, TxPrevout>,
    order: std::collections::VecDeque<OutPoint>,
    capacity: usize,
}

impl PrevoutCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: std::collections::VecDeque::new(),
            capacity,
        }
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
        if let std::collections::hash_map::Entry::Occupied(mut e) = self.map.entry(key) {
            e.insert(value);
            return;
        }
        self.map.insert(key, value);
        self.order.push_back(key);
        if self.map.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
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

pub fn process_prevouts(db: Arc<DB>, block: &Block, data_to_write: &mut Vec<ProcessedData>, mut cache: Option<&mut PrevoutCache>) -> anyhow::Result<HashMap<OutPoint, TxPrevout>> {
    // Pre-allocate prevouts HashMap based on the total number of outputs in the block.
    let outputs_capacity: usize = block.txs.iter().map(|tx| tx.value.outputs.len()).sum();
    let mut prevouts: HashMap<OutPoint, TxPrevout> = HashMap::with_capacity(outputs_capacity);

    for tx in &block.txs {
        let txid = tx.hash;
        for (vout, txout) in tx.value.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid: txid.into(),
                vout: vout as u32,
            };
            let tx_out = TxOut {
                value: txout.out.value,
                script_pubkey: ScriptBuf::from_bytes(txout.out.script_pubkey.clone()),
            };

            if tx_out.script_pubkey.is_provably_unspendable() {
                continue;
            }

            prevouts.insert(outpoint, tx_out.into());
        }
    }

    let txids_keys = block
        .txs
        .iter()
        .filter(|tx| !tx.value.is_coinbase())
        .flat_map(|tx| tx.value.inputs.iter().map(|x| x.outpoint))
        .unique()
        .collect_vec();

    let mut result = HashMap::with_capacity(txids_keys.len());

    if !txids_keys.is_empty() {
        let mut missing_keys = Vec::new();

        // Try cache first
        if let Some(ref cache) = cache {
            for key in &txids_keys {
                if let Some(val) = cache.get(key) {
                    result.insert(*key, val);
                } else {
                    missing_keys.push(*key);
                }
            }
        } else {
            missing_keys = txids_keys.clone();
        }

        if !missing_keys.is_empty() {
            let from_db = db.prevouts.multi_get(missing_keys.iter());

            for (key, maybe_val) in missing_keys.iter().zip(from_db) {
                match maybe_val {
                    Some(val) => {
                        if let Some(cache) = cache.as_deref_mut() {
                            cache.insert(*key, val);
                        }
                        result.insert(*key, val);
                    }
                    None => {
                        if let Some(value) = prevouts.get(key) {
                            if let Some(cache) = cache.as_deref_mut() {
                                cache.insert(*key, *value);
                            }
                            result.insert(*key, *value);
                        } else {
                            return Err(anyhow::anyhow!("Missing prevout for key {}. Block: {}", key, block.header.hash));
                        }
                    }
                }
            }
        }
    }

    if let Some(ref mut cache) = cache {
        cache.insert_block_outputs(prevouts.iter().map(|(k, v)| (*k, *v)));
    }

    data_to_write.push(ProcessedData::Prevouts {
        to_write: prevouts,
        to_remove: txids_keys,
    });

    Ok(result)
}
