#![allow(clippy::uninlined_format_args)]

#[macro_use]
extern crate tracing;

use bellscoin::hashes::{Hash, sha256d};
use dutils::{error::ContextWrapper, wait_token::WaitToken};
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
        BlockId, CoinType,
        checkpoint::CheckPoint,
        parser::{ChainOptions, ChainStorage},
        proto::{Hashed, address_to_fullhash},
    },
    utils::BlockHeightRange,
};

mod blockchain;
mod utils;

pub use blockchain::{
    LoadBlocks, LoadBlocksArgs,
    proto::{self, ScriptType},
};
pub use utils::Auth;

const BOUNDED_CHANNEL_SIZE: usize = 30;

type Result<T> = std::result::Result<T, anyhow::Error>;

pub struct BlockEvent {
    pub id: BlockId,
    pub block: blockchain::proto::block::Block,
    pub reorg_len: usize,
    pub tip: u64,
}

pub struct Indexer {
    pub path: Option<String>,
    pub index_dir_path: Option<String>,
    pub coin: String,
    pub rpc_url: String,
    pub rpc_auth: Auth,
    pub token: WaitToken,
    pub last_height: u32,
    pub reorg_max_len: usize,
}

impl Indexer {
    pub fn parse_blocks(self: Arc<Self>) -> kanal::Receiver<BlockEvent> {
        let (tx, rx) = kanal::bounded::<BlockEvent>(BOUNDED_CHANNEL_SIZE);

        std::thread::spawn(move || {
            let coin = CoinType::from_str(&self.coin).unwrap();
            let mut last_height = if self.last_height == 0 { 0 } else { self.last_height + 1 } as u64;

            let mut chain = ChainStorage::new(&ChainOptions::new(self.path.as_deref(), self.index_dir_path.as_deref(), coin, self.last_height)).unwrap();

            let max_height = chain.max_height();

            for height in last_height..=max_height {
                if self.token.is_cancelled() {
                    return;
                }

                let Some(block) = chain.get_block(height).unwrap() else {
                    break;
                };

                if tx
                    .send(BlockEvent {
                        id: BlockId { height, hash: block.header.hash },
                        block,
                        reorg_len: 0,
                        tip: max_height,
                    })
                    .is_err()
                {
                    return;
                };
            }

            let client = utils::Client::new(&self.rpc_url, self.rpc_auth.clone(), coin, self.token.clone()).unwrap();

            let mut checkpoint = match chain.complete() {
                Some(v) => v,
                None => {
                    last_height = last_height.saturating_sub(1);
                    let hash = client.get_block_hash(last_height).unwrap();
                    CheckPoint::new(BlockId { height: last_height, hash })
                }
            };

            while checkpoint.height() <= last_height {
                let height = checkpoint.height() + 1;
                let hash = client.get_block_hash(height).unwrap();
                checkpoint = checkpoint.insert(BlockId { height, hash });
            }

            while !self.token.is_cancelled() {
                let mut reorg_counter = 0;
                let best_hash = client.get_best_block_hash().unwrap();

                if best_hash != checkpoint.hash() {
                    loop {
                        if reorg_counter > self.reorg_max_len {
                            panic!("Reorg chain is too long");
                        }

                        let hash = checkpoint.hash();
                        match client.get_block_info(&hash) {
                            Ok(v) if v.confirmations < 0 => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                continue;
                            }
                            Err(_) => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                continue;
                            }
                            _ => {}
                        };

                        let best_height = client.get_block_info(&best_hash).unwrap().height as u64;

                        while checkpoint.height() < best_height {
                            let next_height = checkpoint.height() + 1;
                            let next_hash = client.get_block_hash(next_height).unwrap();
                            let block = client.get_block(&next_hash).unwrap();

                            Self::check_order(checkpoint.hash(), &block.header);
                            checkpoint = checkpoint.insert(BlockId {
                                height: next_height,
                                hash: next_hash,
                            });

                            if tx
                                .send(BlockEvent {
                                    block,
                                    id: BlockId {
                                        height: next_height,
                                        hash: next_hash,
                                    },
                                    reorg_len: reorg_counter,
                                    tip: best_height,
                                })
                                .is_err()
                            {
                                return;
                            };

                            reorg_counter = 0;
                        }

                        break;
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
        });

        rx
    }

    pub fn to_scripthash(&self, address: &str, script_type: ScriptType) -> Result<sha256d::Hash> {
        let coin = CoinType::from_str(&self.coin).anyhow_with("Unsupported coin")?;
        address_to_fullhash(address, script_type, coin)
    }

    #[inline]
    fn check_order(last_sent_hash: sha256d::Hash, header: &Hashed<proto::header::BlockHeader>) {
        if last_sent_hash != header.value.prev_hash {
            panic!("Invalid blocks order. Expected {} got {}", last_sent_hash, header.value.prev_hash);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::proto::header::BlockHeader;

    use super::*;

    fn test_block_id(height: u64) -> BlockId {
        BlockId {
            height,
            hash: sha256d::Hash::from_byte_array([height as u8; 32]),
        }
    }

    #[test]
    fn test_reorg() {
        let blocks = [test_block_id(0), test_block_id(1), test_block_id(2), test_block_id(3), test_block_id(4), test_block_id(5)];
        let mut checkpoint = CheckPoint::from_block_ids(blocks).unwrap();

        let best_block_id = BlockId {
            height: 3,
            hash: sha256d::Hash::from_byte_array([6; 32]),
        };

        let mut reorg_counter = 0;

        if best_block_id.hash != checkpoint.hash() {
            while checkpoint.height() >= best_block_id.height {
                reorg_counter += 1;
                checkpoint = checkpoint.prev().unwrap();
                continue;
            }

            let best_height = checkpoint.height();

            while checkpoint.height() < best_height {
                let next_height = checkpoint.height() + 1;
                let next_hash = blocks.get(next_height as usize - 1).unwrap().hash;
                checkpoint = checkpoint.insert(BlockId {
                    height: next_height,
                    hash: next_hash,
                });

                Indexer::check_order(
                    checkpoint.hash(),
                    &Hashed {
                        hash: blocks.get(checkpoint.height() as usize - 1).map(|x| x.hash).unwrap_or(sha256d::Hash::all_zeros()),
                        value: BlockHeader {
                            bits: 0,
                            merkle_root: sha256d::Hash::all_zeros(),
                            nonce: 0,
                            prev_hash: checkpoint.hash(),
                            timestamp: 0,
                            version: 0,
                        },
                    },
                );

                reorg_counter = 0;
            }
        }

        assert_eq!(reorg_counter, 3);
        assert_eq!(checkpoint.height(), best_block_id.height - 1);
        let next_height = checkpoint.height() + 1;
        assert_eq!(next_height, best_block_id.height);
    }
}
