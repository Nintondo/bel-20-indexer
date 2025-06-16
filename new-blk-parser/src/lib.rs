#[macro_use]
extern crate tracing;

use bellscoin::hashes::{Hash, sha256d};
use dutils::{error::ContextWrapper, wait_token::WaitToken};
use kanal::Receiver;
use num_traits::Zero;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::{
    borrow::BorrowMut,
    collections::{HashMap, VecDeque},
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

const BOUNDED_CHANNEL_SIZE: usize = 50;

type Result<T> = std::result::Result<T, anyhow::Error>;

pub struct BlockEvent {
    pub id: BlockId,
    pub block: blockchain::proto::block::Block,
    pub reorg_len: usize,
    pub tip: u64,
}

pub struct Indexer {
    pub path: String,
    pub coin: String,
    pub rpc_url: String,
    pub rpc_auth: Auth,
    pub token: WaitToken,
    pub last_height: u32,
    pub reorg_max_len: usize,
    pub index_dir_path: String,
}

impl Indexer {
    pub fn parse_blocks(self: Arc<Self>) -> Receiver<BlockEvent> {
        let (tx, rx) = kanal::bounded::<BlockEvent>(BOUNDED_CHANNEL_SIZE);

        let this = self.clone();

        std::thread::spawn(move || {
            let coin = CoinType::from_str(&this.coin).unwrap();
            let mut last_height = this
                .last_height
                .is_zero()
                .then_some(0)
                .unwrap_or(this.last_height + 1) as u64;

            let mut chain = ChainStorage::new(&ChainOptions::new(
                &this.path,
                &this.index_dir_path,
                coin,
                this.last_height,
            ))
            .unwrap();

            let max_height = chain.max_height();

            for height in last_height..=max_height {
                if this.token.is_cancelled() {
                    return;
                }

                let Some(block) = chain.get_block(height).unwrap() else {
                    break;
                };
                if tx
                    .send(BlockEvent {
                        id: BlockId {
                            height,
                            hash: block.header.hash,
                        },
                        block,
                        reorg_len: 0,
                        tip: max_height,
                    })
                    .is_err()
                {
                    return;
                };
            }

            let client = utils::Client::new(
                &this.rpc_url,
                this.rpc_auth.clone(),
                coin,
                this.token.clone(),
            )
            .unwrap();

            let mut checkpoint = match chain.complete() {
                Some(v) => v,
                None => {
                    last_height -= 1;
                    let hash = client.get_block_hash(last_height).unwrap();
                    CheckPoint::new(BlockId {
                        height: last_height,
                        hash,
                    })
                }
            };

            while checkpoint.height() < last_height {
                let height = checkpoint.height() + 1;
                let hash = client.get_block_hash(height).unwrap();
                checkpoint = checkpoint.insert(BlockId { height, hash });
            }

            while !this.token.is_cancelled() {
                let mut reorg_counter = 0;
                let mut new_blocks = VecDeque::new();
                let best_hash = client.get_best_block_hash().unwrap();

                if best_hash != checkpoint.hash() {
                    loop {
                        if reorg_counter > this.reorg_max_len {
                            panic!("Reorg chain is too long");
                        }

                        let hash = checkpoint.hash();
                        let hash = match client.get_block_info(&hash) {
                            Ok(v) if v.confirmations < 0 => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                continue;
                            }
                            Ok(v) => v.hash,
                            Err(_) => {
                                reorg_counter += 1;
                                checkpoint = checkpoint.prev().unwrap();
                                continue;
                            }
                        };

                        let mut last_hash = best_hash;

                        while last_hash != hash {
                            let block = client.get_block(&last_hash).unwrap();
                            last_hash = block.header.value.prev_hash;
                            new_blocks.push_front((block.header.hash, block));
                        }

                        break;
                    }

                    let tip_height = checkpoint.height() + new_blocks.len() as u64;

                    for (hash, block) in new_blocks {
                        let id = BlockId {
                            height: checkpoint.height() + 1,
                            hash,
                        };
                        checkpoint = checkpoint.insert(id);

                        if tx
                            .send(BlockEvent {
                                block,
                                id,
                                reorg_len: reorg_counter,
                                tip: tip_height,
                            })
                            .is_err()
                        {
                            return;
                        };
                        reorg_counter = 0;
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
}
