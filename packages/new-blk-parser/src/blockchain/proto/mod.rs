use super::*;
use std::io::Write;
pub use hashbrown;

pub mod block;
pub mod header;
pub mod script;
pub mod tx;
pub mod varuint;

pub use script::{ScriptType, address_to_fullhash};

/// Trait to serialize defined structures
pub trait ToRaw {
    fn to_bytes(&self) -> Vec<u8>;
}

/// Trait for types that can stream their serialized form into a SHA256d engine
/// without allocating a temporary Vec.
pub trait Hashable {
    fn hash_to_engine<W: Write>(&self, engine: &mut W);
}

/// Wrapper to hold a 32 byte verification hash along the data type T
pub struct Hashed<T> {
    pub hash: sha256d::Hash,
    pub value: T,
}

impl<T: Hashable> Hashed<T> {
    /// encapsulates T and creates double sha256 as hash
    #[inline]
    pub fn double_sha256(value: T) -> Hashed<T> {
        use crate::timing::BLOCK_READ_METRICS;
        let start = std::time::Instant::now();
        let mut engine = sha256d::Hash::engine();
        value.hash_to_engine(&mut engine);
        let hash = sha256d::Hash::from_engine(engine);
        BLOCK_READ_METRICS.record_hash(start.elapsed());
        Hashed { hash, value }
    }
}

impl<T: fmt::Debug> fmt::Debug for Hashed<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Hashed").field("hash", &self.hash).field("value", &self.value).finish()
    }
}

/// https://en.bitcoin.it/wiki/Merged_mining_specification#Merkle_Branch
pub struct MerkleBranch {
    pub hashes: Vec<[u8; 32]>,
    // Bitmask of which side of the merkle hash function the branch_hash element should go on.
    // Zero means it goes on the right, One means on the left.
    // It is equal to the index of the starting hash within the widest level
    // of the merkle tree for this merkle branch.
    pub side_mask: u32,
}

impl MerkleBranch {
    #[inline]
    pub fn new(hashes: Vec<[u8; 32]>, side_mask: u32) -> Self {
        Self { hashes, side_mask }
    }
}
