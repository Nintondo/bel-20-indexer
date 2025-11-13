use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use bellscoin::hashes::{sha256d, Hash};

use nint_blk::proto::block::Block;
use nint_blk::proto::header::BlockHeader;
use nint_blk::proto::Hashed;
use nint_blk::RpcRead;
use nint_blk::GetBlockResult;

#[derive(Clone)]
pub struct MockRpc {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    best: sha256d::Hash,
    heights: BTreeMap<u64, sha256d::Hash>,
    blocks: HashMap<sha256d::Hash, Block>,
    infos: HashMap<sha256d::Hash, GetBlockResult>,
}

impl MockRpc {
    pub fn default() -> Self { Self { inner: Arc::new(Mutex::new(Inner::default())) } }
    pub fn with_best(self, h: sha256d::Hash) -> Self { self.inner.lock().unwrap().best = h; self }
    pub fn set_height(&self, height: u64, hash: sha256d::Hash) { self.inner.lock().unwrap().heights.insert(height, hash); }
    pub fn set_block(&self, hash: sha256d::Hash, block: Block) { self.inner.lock().unwrap().blocks.insert(hash, block); }
    pub fn set_info(&self, hash: sha256d::Hash, info: GetBlockResult) { self.inner.lock().unwrap().infos.insert(hash, info); }
}

impl Default for Inner {
    fn default() -> Self {
        Self { best: sha256d::Hash::all_zeros(), heights: BTreeMap::new(), blocks: HashMap::new(), infos: HashMap::new() }
    }
}

impl RpcRead for MockRpc {
    fn get_block(&self, hash: &sha256d::Hash) -> nint_blk::RpcResult<Block> {
        let maybe = self.inner.lock().unwrap().blocks.get(hash).map(|b| {
            let header = Hashed { hash: b.header.hash, value: BlockHeader { ..b.header.value.clone() } };
            Block { size: b.size, header, aux_pow_extension: None, tx_count: b.tx_count.clone(), txs: vec![] }
        });
        if let Some(block) = maybe {
            Ok(block)
        } else {
            Err(nint_blk::RpcError::Cancelled)
        }
    }

    fn get_block_info(&self, hash: &sha256d::Hash) -> nint_blk::RpcResult<GetBlockResult> {
        self.inner
            .lock()
            .unwrap()
            .infos
            .get(hash)
            .cloned()
            .ok_or_else(|| nint_blk::RpcError::Cancelled)
    }

    fn get_block_hash(&self, height: u64) -> nint_blk::RpcResult<sha256d::Hash> {
        self.inner
            .lock()
            .unwrap()
            .heights
            .get(&height)
            .cloned()
            .ok_or_else(|| nint_blk::RpcError::Cancelled)
    }

    fn get_best_block_hash(&self) -> nint_blk::RpcResult<sha256d::Hash> {
        Ok(self.inner.lock().unwrap().best)
    }
}
