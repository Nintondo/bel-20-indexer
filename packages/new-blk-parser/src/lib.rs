#[macro_use]
extern crate tracing;

use bellscoin::hashes::{Hash, sha256d};
use dutils::{error::ContextWrapper, wait_token::WaitToken};
use num_traits::Zero;
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
        proto::address_to_fullhash,
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
            let mut last_height = self.last_height.is_zero().then_some(0).unwrap_or(self.last_height + 1) as u64;

            let mut chain = ChainStorage::new(&ChainOptions::new(
                self.path.as_ref().map(|x| x.as_str()),
                self.index_dir_path.as_ref().map(|x| x.as_str()),
                coin,
                self.last_height,
            ))
            .unwrap();

            let max_height = chain.max_height();

            let mut last_sent_hash: Option<sha256d::Hash> = None;

            for height in last_height..=max_height {
                if self.token.is_cancelled() {
                    return;
                }

                let Some(block) = chain.get_block(height).unwrap() else {
                    break;
                };

                Self::check_order(&mut last_sent_hash, &block);
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

            while checkpoint.height() < last_height {
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
                            let next_height = checkpoint.height() - reorg_counter as u64 + 1;
                            let next_hash = client.get_block_hash(next_height).unwrap();
                            checkpoint = checkpoint.insert(BlockId {
                                height: next_height,
                                hash: next_hash,
                            });
                            let block = client.get_block(&next_hash).unwrap();
                            Self::check_order(&mut last_sent_hash, &block);
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
    fn check_order(last_sent_hash: &mut Option<sha256d::Hash>, block: &proto::block::Block) {
        if last_sent_hash.is_none() {
            let _ = last_sent_hash.insert(block.header.hash);
        } else {
            if last_sent_hash.unwrap() != block.header.value.prev_hash {
                panic!("Invalid blocks order");
            }
            let _ = last_sent_hash.insert(block.header.hash);
        }
    }
}
