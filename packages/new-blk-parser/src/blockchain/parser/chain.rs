use blockchain::{block_id::BlockId, checkpoint::CheckPoint};

use super::*;

use blk_file::BlkFile;
use itertools::Itertools;
use parser::index::ChainIndex;
use proto::block::Block;

/// Manages the index and data of longest valid chain
pub struct ChainStorage {
    pub chain_index: ChainIndex,
    coin: CoinType,
    blk_files: Option<HashMap<u64, BlkFile>>, // maps blk_index to BlkFile
}

impl ChainStorage {
    pub fn new(options: &ChainOptions) -> Result<Self> {
        Ok(Self {
            coin: options.coin,
            chain_index: ChainIndex::new(options)?,
            blk_files: options.blockchain_dir.as_ref().map(|x| BlkFile::from_path(x.as_path())).transpose()?,
        })
    }

    /// Returns the block at the given height
    pub fn get_block(&mut self, height: u64) -> Result<Option<Block>> {
        // Read block
        let block_meta = match self.chain_index.get(height) {
            Some(block_meta) => block_meta,
            None => return Ok(None),
        };

        let Some(blk_files) = &mut self.blk_files else { return Ok(None) };

        let blk_file = blk_files.get_mut(&block_meta.blk_index).anyhow_with("Block file for block not found")?;
        let block = blk_file.read_block(block_meta.data_offset, self.coin).anyhow_with("Unable to read block")?;

        // Check if blk file can be closed
        if height >= self.chain_index.max_height_by_blk(block_meta.blk_index) {
            blk_file.close()
        }

        Ok(Some(block))
    }

    #[inline]
    pub(crate) const fn max_height(&self) -> u64 {
        self.chain_index.max_height()
    }

    pub fn complete(self) -> Option<CheckPoint> {
        let iterator = self
            .chain_index
            .block_index
            .into_iter()
            .sorted_unstable_by_key(|x| x.0)
            .map(|(k, v)| BlockId { hash: v.block_hash, height: k });

        CheckPoint::from_block_ids(iterator).ok()
    }
}
