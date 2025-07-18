use super::*;

use std::result::Result;
use std::sync::Arc;

use block_id::BlockId;

/// A checkpoint is a node of a reference-counted linked list of [`BlockId`]s.
///
/// Checkpoints are cheaply cloneable and are useful to find the agreement point between two sparse
/// block chains.
#[derive(Debug, Clone)]
pub struct CheckPoint(Arc<CPInner>);

/// The internal contents of [`CheckPoint`].
#[derive(Debug, Clone)]
struct CPInner {
    /// Block id (hash and height).
    block: BlockId,
    /// Previous checkpoint (if any).
    prev: Option<Arc<CPInner>>,
}

/// When a `CPInner` is dropped we need to go back down the chain and manually remove any
/// no-longer referenced checkpoints. Letting the default rust dropping mechanism handle this
/// leads to recursive logic and stack overflows
///
/// https://github.com/bitcoindevkit/bdk/issues/1634
impl Drop for CPInner {
    fn drop(&mut self) {
        // Take out `prev` so its `drop` won't be called when this drop is finished
        let mut current = self.prev.take();
        while let Some(arc_node) = current {
            // Get rid of the Arc around `prev` if we're the only one holding a ref
            // So the `drop` on it won't be called when the `Arc` is dropped.
            //
            // that no recursive drop calls can happen even with multiple threads.
            match Arc::into_inner(arc_node) {
                Some(mut node) => {
                    // Keep going backwards
                    current = node.prev.take();
                    // Don't call `drop` on `CPInner` since that risks it becoming recursive.
                    core::mem::forget(node);
                }
                None => break,
            }
        }
    }
}

impl PartialEq for CheckPoint {
    fn eq(&self, other: &Self) -> bool {
        let self_cps = self.iter().map(|cp| cp.block_id());
        let other_cps = other.iter().map(|cp| cp.block_id());
        self_cps.eq(other_cps)
    }
}

impl CheckPoint {
    /// Construct a new base block at the front of a linked list.
    pub fn new(block: BlockId) -> Self {
        Self(Arc::new(CPInner { block, prev: None }))
    }

    /// Construct a checkpoint from a list of [`BlockId`]s in ascending height order.
    ///
    /// # Errors
    ///
    /// This method will error if any of the follow occurs:
    ///
    /// - The `blocks` iterator is empty, in which case, the error will be `None`.
    /// - The `blocks` iterator is not in ascending height order.
    /// - The `blocks` iterator contains multiple [`BlockId`]s of the same height.
    ///
    /// The error type is the last successful checkpoint constructed (if any).
    pub fn from_block_ids(
        block_ids: impl IntoIterator<Item = BlockId>,
    ) -> Result<Self, Option<Self>> {
        let mut blocks = block_ids.into_iter();
        let mut acc = CheckPoint::new(blocks.next().ok_or(None)?);
        for id in blocks {
            acc = acc.push(id).map_err(Some)?;
        }
        Ok(acc)
    }

    /// Puts another checkpoint onto the linked list representing the blockchain.
    ///
    /// Returns an `Err(self)` if the block you are pushing on is not at a greater height that the
    /// one you are pushing on to.
    pub fn push(self, block: BlockId) -> Result<Self, Self> {
        if self.height() < block.height {
            Ok(Self(Arc::new(CPInner {
                block,
                prev: Some(self.0),
            })))
        } else {
            Err(self)
        }
    }

    /// Extends the checkpoint linked list by a iterator of block ids.
    ///
    /// Returns an `Err(self)` if there is block which does not have a greater height than the
    /// previous one.
    pub fn extend(self, blocks: impl IntoIterator<Item = BlockId>) -> Result<Self, Self> {
        let mut curr = self.clone();
        for block in blocks {
            curr = curr.push(block).map_err(|_| self.clone())?;
        }
        Ok(curr)
    }

    /// Get the [`BlockId`] of the checkpoint.
    pub fn block_id(&self) -> BlockId {
        self.0.block
    }

    /// Get the height of the checkpoint.
    pub fn height(&self) -> u64 {
        self.0.block.height
    }

    /// Get the block hash of the checkpoint.
    pub fn hash(&self) -> sha256d::Hash {
        self.0.block.hash
    }

    /// Get the previous checkpoint in the chain
    pub fn prev(&self) -> Option<CheckPoint> {
        self.0.prev.clone().map(CheckPoint)
    }

    /// Iterate from this checkpoint in descending height.
    pub fn iter(&self) -> CheckPointIter {
        self.clone().into_iter()
    }

    /// Inserts `block_id` at its height within the chain.
    ///
    /// The effect of `insert` depends on whether a height already exists. If it doesn't the
    /// `block_id` we inserted and all pre-existing blocks higher than it will be re-inserted after
    /// it. If the height already existed and has a conflicting block hash then it will be purged
    /// along with all block following it. The returned chain will have a tip of the `block_id`
    /// passed in. Of course, if the `block_id` was already present then this just returns `self`.
    ///
    /// # Panics
    ///
    /// This panics if called with a genesis block that differs from that of `self`.
    #[must_use]
    pub fn insert(self, block_id: BlockId) -> Self {
        let mut cp = self.clone();
        let mut tail = vec![];
        let base = loop {
            if cp.height() == block_id.height {
                if cp.hash() == block_id.hash {
                    return self;
                }
                assert_ne!(cp.height(), 0, "cannot replace genesis block");
                // if we have a conflict we just return the inserted block because the tail is by
                // implication invalid.
                tail = vec![];
                break cp.prev().expect("can't be called on genesis block");
            }

            if cp.height() < block_id.height {
                break cp;
            }

            tail.push(cp.block_id());
            cp = cp.prev().expect("will break before genesis block");
        };

        base.extend(core::iter::once(block_id).chain(tail.into_iter().rev()))
            .expect("tail is in order")
    }
}

/// Iterates over checkpoints backwards.
pub struct CheckPointIter {
    current: Option<Arc<CPInner>>,
}

impl Iterator for CheckPointIter {
    type Item = CheckPoint;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current.clone()?;
        self.current.clone_from(&current.prev);
        Some(CheckPoint(current))
    }
}

impl IntoIterator for CheckPoint {
    type Item = CheckPoint;
    type IntoIter = CheckPointIter;

    fn into_iter(self) -> Self::IntoIter {
        CheckPointIter {
            current: Some(self.0),
        }
    }
}
