#![allow(clippy::uninlined_format_args)]

#[macro_use]
extern crate tracing;

use bellscoin::hashes::{Hash, sha256, sha256d};
use dutils::{error::ContextWrapper, wait_token::WaitToken};
use kanal::{SendError, Sender};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::{
    borrow::BorrowMut,
    collections::HashMap,
    convert::{From, TryInto},
    fmt::{self, Write},
    fs::{self, DirEntry, File},
    io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use crate::{
    blockchain::{
        checkpoint::CheckPoint,
        parser::{ChainOptions, ChainStorage},
        proto::{Hashed, address_to_fullhash},
    },
    utils::BlockHeightRange,
};

mod blockchain;
mod startup;
pub use startup::validate_local_paths;
use startup::{validate_local_checkpoint, validate_tip};
mod utils;

pub use blockchain::{
    BlockId, CoinType, LoadBlocks, LoadBlocksArgs,
    proto::{self, ScriptType},
};
pub use utils::{Auth, Client};

const BOUNDED_CHANNEL_SIZE: usize = 30;

type Result<T> = std::result::Result<T, anyhow::Error>;

pub struct BlockEvent {
    pub id: BlockId,
    pub block: blockchain::proto::block::Block,
    pub reorg_len: usize,
    pub tip: u64,
}

pub enum IndexerEvent {
    Block(BlockEvent),
    Synced(BlockId),
    InvalidTip(String),
}

pub struct Indexer {
    pub path: Option<String>,
    pub index_dir_path: Option<String>,
    pub coin: CoinType,
    pub token: WaitToken,
    pub last_block: BlockId,
    pub reorg_max_len: usize,
    pub client: Arc<Client>,
}

trait SendChecked {
    fn send_checked(&self, event: BlockEvent, last_sent_hash: &mut sha256d::Hash) -> std::result::Result<(), SendError>;
}

impl SendChecked for Sender<IndexerEvent> {
    fn send_checked(&self, event: BlockEvent, last_sent_hash: &mut sha256d::Hash) -> std::result::Result<(), SendError> {
        check_ordering(last_sent_hash, &event.block.header);
        self.send(IndexerEvent::Block(event))
    }
}

#[inline]
fn check_ordering(last_sent_hash: &mut sha256d::Hash, header: &Hashed<proto::header::BlockHeader>) {
    if *last_sent_hash != header.value.prev_hash {
        panic!("Invalid blocks order. Expected {} got {}", last_sent_hash, header.value.prev_hash);
    }
    *last_sent_hash = header.hash;
}

impl Indexer {
    pub fn parse_blocks(self: Arc<Self>) -> kanal::Receiver<IndexerEvent> {
        let (tx, rx) = kanal::bounded::<IndexerEvent>(BOUNDED_CHANNEL_SIZE);

        std::thread::spawn(move || {
            if let Err(error) = validate_local_paths(self.path.as_deref(), self.index_dir_path.as_deref()) {
                let _ = tx.send(IndexerEvent::InvalidTip(error.to_string()));
                return;
            }
            if self.last_block.hash != sha256d::Hash::all_zeros() {
                let check = self
                    .client
                    .get_block_hash(self.last_block.height)
                    .map_err(anyhow::Error::from)
                    .and_then(|hash| validate_tip(self.last_block, hash));
                if let Err(error) = check {
                    let _ = tx.send(IndexerEvent::InvalidTip(format!(
                        "Stored tip is not canonical; exports disabled, reindex required: {error:#}"
                    )));
                    return;
                }
            }
            let mut last_height = {
                let last = self.last_block.height;
                if last == 0 && self.last_block.hash == sha256d::Hash::all_zeros() {
                    last
                } else {
                    last + 1
                }
            };
            let mut last_hash = self.last_block.hash;
            let mut emitted_tip = self.last_block;

            let mut chain = ChainStorage::new(&ChainOptions::new(
                self.path.as_deref(),
                self.index_dir_path.as_deref(),
                self.coin,
                self.last_block.height as u32,
            ))
            .unwrap();

            let max_height = chain.max_height();

            for height in last_height..=max_height {
                if self.token.is_cancelled() {
                    return;
                }

                let Some(block) = chain.get_block(height).unwrap() else {
                    break;
                };

                let event = BlockEvent {
                    id: BlockId { height, hash: block.header.hash },
                    block,
                    reorg_len: 0,
                    tip: max_height,
                };

                emitted_tip = event.id;
                if tx.send_checked(event, &mut last_hash).is_err() {
                    return;
                };
            }

            let mut checkpoint = match chain.complete() {
                Some(v) => {
                    if let Err(error) = validate_local_checkpoint(v.block_id(), emitted_tip, self.last_block) {
                        let _ = tx.send(IndexerEvent::InvalidTip(error.to_string()));
                        return;
                    }
                    // A copied index may lag the RPC-verified saved tip. Never
                    // advance beyond emitted data, nor regress to that stale copy.
                    if v.height() < emitted_tip.height { CheckPoint::new(emitted_tip) } else { v }
                }
                None => {
                    last_height = last_height.saturating_sub(1);
                    // Keep the saved identity, verified above, instead of silently
                    // attaching old balances to the node's replacement block.
                    let hash = if self.last_block.hash != sha256d::Hash::all_zeros() {
                        self.last_block.hash
                    } else {
                        self.client.get_block_hash(last_height).unwrap()
                    };
                    last_hash = hash;
                    CheckPoint::new(BlockId { height: last_height, hash })
                }
            };

            while checkpoint.height() < last_height.saturating_sub(1) {
                let height = checkpoint.height() + 1;
                let hash = self.client.get_block_hash(height).unwrap();
                checkpoint = checkpoint.insert(BlockId { height, hash });
            }

            let mut announced_synced = false;
            while !self.token.is_cancelled() {
                let mut reorg_counter = 0;
                let best_hash = self.client.get_best_block_hash().unwrap();

                if best_hash != checkpoint.hash() {
                    announced_synced = false;
                    loop {
                        if reorg_counter > self.reorg_max_len {
                            panic!("Reorg chain is too long");
                        }

                        let hash = checkpoint.hash();
                        match self.client.get_block_info(&hash) {
                            Ok(v) if v.confirmations < 0 => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                last_hash = checkpoint.hash();
                                continue;
                            }
                            Err(_) => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                last_hash = checkpoint.hash();
                                continue;
                            }
                            _ => {}
                        };

                        let best_height = self.client.get_block_info(&best_hash).unwrap().height as u64;

                        while checkpoint.height() < best_height {
                            let next_height = checkpoint.height() + 1;
                            let next_hash = self.client.get_block_hash(next_height).unwrap();
                            let block = self.client.get_block(&next_hash).unwrap();

                            // Guard if reorg happened in the mid of loop
                            if block.header.value.prev_hash != checkpoint.hash() {
                                break;
                            }

                            let event = BlockEvent {
                                block,
                                id: BlockId {
                                    height: next_height,
                                    hash: next_hash,
                                },
                                reorg_len: reorg_counter,
                                tip: best_height,
                            };

                            if tx.send_checked(event, &mut last_hash).is_err() {
                                return;
                            };

                            checkpoint = checkpoint.insert(BlockId {
                                height: next_height,
                                hash: next_hash,
                            });

                            reorg_counter = 0;
                        }

                        break;
                    }
                } else {
                    if !announced_synced {
                        // This marker follows all queued blocks, so the consumer
                        // only advertises readiness after committing that tip.
                        let tip = BlockId {
                            height: checkpoint.height(),
                            hash: checkpoint.hash(),
                        };
                        if tx.send(IndexerEvent::Synced(tip)).is_err() {
                            return;
                        }
                        announced_synced = true;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
        });

        rx
    }

    pub fn to_scripthash(&self, address: &str, script_type: ScriptType) -> Result<sha256::Hash> {
        address_to_fullhash(address, script_type, self.coin)
    }
}

#[cfg(test)]
mod tests;
