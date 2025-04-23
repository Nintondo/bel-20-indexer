use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    marker::PhantomData,
    ops::ControlFlow,
    path::PathBuf,
};

use bellscoin::hashes::{sha256d, Hash};
use blk_index_to_blk_path::*;
use blk_recap::BlkRecap;
use kanal::Sender;

mod blk_index_to_blk_path;
mod blk_index_to_blk_recap;
mod blk_metadata;
mod blk_recap;
mod block_state;
mod utils;

use blk_index_to_blk_recap::*;
use blk_metadata::*;
use block_state::*;
use serde::{Deserialize, Serialize};
use utils::*;

pub trait InnerBlockHash: Sized + Send + Sync {
    type Error: std::fmt::Debug;

    fn inner_block_hash(&self) -> sha256d::Hash;
    fn consensus_decode<C: std::io::Read + ?Sized>(cursor: &mut C) -> Result<Self, Self::Error>;
}

impl InnerBlockHash for bellscoin::Block {
    type Error = bellscoin::consensus::encode::Error;

    fn inner_block_hash(&self) -> sha256d::Hash {
        let bytes = *bellscoin::hashes::Hash::as_byte_array(self.block_hash().as_raw_hash());
        sha256d::Hash::from_byte_array(bytes)
    }

    fn consensus_decode<C: std::io::Read + ?Sized>(cursor: &mut C) -> Result<Self, Self::Error> {
        bellscoin::consensus::Decodable::consensus_decode(cursor)
    }
}

pub(crate) type Height = u32;
pub(crate) type Confirmations = i32;

pub trait NodeClient: Send + Sync {
    type Error: std::fmt::Debug;

    fn get_block_header_info(
        &self,
        hash: &sha256d::Hash,
    ) -> Result<(Height, Confirmations), Self::Error>;
}

impl NodeClient for bellscoincore_rpc::Client {
    type Error = bellscoincore_rpc::Error;

    fn get_block_header_info(
        &self,
        hash: &sha256d::Hash,
    ) -> Result<(Height, Confirmations), Self::Error> {
        let hash = <bellscoin::BlockHash as bellscoin::hashes::Hash>::from_byte_array(
            hash.to_byte_array(),
        );

        let header: GetBlockHeaderResult = bellscoincore_rpc::RpcApi::call(
            self,
            "getblockheader",
            &[serde_json::to_value(hash)?, true.into()],
        )?;

        Ok((header.height as u32, header.confirmations))
    }
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockHeaderResult {
    pub confirmations: i32,
    pub height: usize,
}

pub struct Parser<T: InnerBlockHash, U: NodeClient> {
    blocks_dir: PathBuf,
    rpc: U,
    magic: [u8; 4],
    _block: PhantomData<T>,
}

impl<T: InnerBlockHash, U: NodeClient> Parser<T, U> {
    pub fn new(blocks_dir: PathBuf, rpc: U, magic: [u8; 4]) -> Self {
        Self {
            blocks_dir,
            rpc,
            magic,
            _block: PhantomData,
        }
    }

    // pub fn get(&self, height: Height) -> T {
    //     let rx = self.parse(Some(height), Some(height));
    //     let (_, block, _) = rx.recv().unwrap();
    //     block
    // }

    /// Returns a crossbeam channel receiver that receives `(Height, Block, BlockHash)` tuples from an **inclusive** range (`start` and `end`)
    pub fn parse(
        &self,
        send_height_block_hash: Sender<(u32, T, sha256d::Hash)>,
        start: Option<Height>,
        end: Option<Height>,
    ) {
        let blocks_dir = self.blocks_dir.as_path();

        // let (send_height_block_hash, recv_height_block_hash) = bounded(BOUND_CAP);

        let blk_index_to_blk_path = BlkIndexToBlkPath::scan(blocks_dir);

        let (mut blk_index_to_blk_recap, blk_index) =
            BlkIndexToBlkRecap::import(blocks_dir, &blk_index_to_blk_path, start);

        let magic = self.magic;

        let mut current_height = start.unwrap_or_default();
        let mut future_blocks = BTreeMap::default();

        blk_index_to_blk_path
            .range(blk_index..)
            .try_for_each(move |(blk_index, blk_path)| {
                let blk_index = *blk_index;

                let blk_metadata = BlkMetadata::new(blk_index, blk_path.as_path());

                let mut blk_bytes_ = fs::read(blk_path).unwrap();
                let blk_bytes = blk_bytes_.as_mut_slice();
                let blk_bytes_len = blk_bytes.len();

                let mut current_4bytes = [0; 4];
                let mut cursor = Cursor::new(blk_bytes);

                while cursor.position() < blk_bytes_len as u64 {
                    cursor.read_exact(&mut current_4bytes).unwrap();

                    if current_4bytes != magic {
                        break;
                    }

                    let mut len_bytes = [0u8; 4];
                    cursor
                        .read_exact(&mut len_bytes)
                        .expect("Invalid length of block");
                    let len = u32::from_le_bytes(len_bytes);

                    let mut block_result = vec![0; len as usize];
                    cursor
                        .read_exact(&mut block_result)
                        .expect("Failed to read block bytes");

                    let mut block = BlockState::<T>::Raw(block_result);
                    block.decode();

                    let BlockState::Decoded(decoded_block) = block else {
                        unreachable!();
                    };

                    let hash = decoded_block.inner_block_hash();
                    let height = match self.rpc.get_block_header_info(&hash) {
                        Ok((height, confirmations)) if confirmations > 0 => height,
                        _ => return ControlFlow::Continue(()),
                    };

                    let len = blk_index_to_blk_recap.tree.len();
                    if blk_metadata.index == len as u16 || blk_metadata.index + 1 == len as u16 {
                        match (len as u16).cmp(&blk_metadata.index) {
                            Ordering::Equal => {
                                if len % 21 == 0 {
                                    blk_index_to_blk_recap.export();
                                }
                            }
                            Ordering::Less => panic!(),
                            Ordering::Greater => {}
                        }

                        blk_index_to_blk_recap
                            .tree
                            .entry(blk_metadata.index)
                            .and_modify(|recap| {
                                if recap.max_height < height {
                                    recap.max_height = height;
                                }
                            })
                            .or_insert(BlkRecap {
                                max_height: height,
                                modified_time: blk_metadata.modified_time,
                            });
                    }

                    let mut opt = if current_height == height {
                        Some((decoded_block, hash))
                    } else {
                        if start.is_none_or(|start| start <= height)
                            && end.is_none_or(|end| end >= height)
                        {
                            future_blocks.insert(height, (decoded_block, hash));
                        }
                        None
                    };

                    while let Some((decoded_block, hash)) = opt.take().or_else(|| {
                        if !future_blocks.is_empty() {
                            future_blocks.remove(&current_height)
                        } else {
                            None
                        }
                    }) {
                        if end.is_some_and(|end| end < current_height) {
                            return ControlFlow::Break(());
                        }

                        let Ok(_) =
                            send_height_block_hash.send((current_height, decoded_block, hash))
                        else {
                            return ControlFlow::Break(());
                        };

                        if end.is_some_and(|end| end == current_height) {
                            return ControlFlow::Break(());
                        }

                        current_height += 1;
                    }
                }
                blk_index_to_blk_recap.export();
                ControlFlow::Continue(())
            });
    }
}
