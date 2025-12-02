use super::*;

/// A reference to a block in the canonical chain.
#[derive(Debug, Clone, PartialEq, Eq, Copy, PartialOrd, Ord, core::hash::Hash)]
pub struct BlockId {
    /// The height of the block.
    pub height: u64,
    /// The hash of the block.
    pub hash: sha256d::Hash,
}

impl Default for BlockId {
    fn default() -> Self {
        Self {
            height: Default::default(),
            hash: sha256d::Hash::all_zeros(),
        }
    }
}

impl From<(u64, sha256d::Hash)> for BlockId {
    fn from((height, hash): (u64, sha256d::Hash)) -> Self {
        Self { height, hash }
    }
}

impl From<BlockId> for (u64, sha256d::Hash) {
    fn from(block_id: BlockId) -> Self {
        (block_id.height, block_id.hash)
    }
}

impl From<(&u64, &sha256d::Hash)> for BlockId {
    fn from((height, hash): (&u64, &sha256d::Hash)) -> Self {
        Self { height: *height, hash: *hash }
    }
}
