use super::*;

mod block_id;
pub mod checkpoint;
pub mod coins;
pub mod parser;
pub mod proto;

pub use block_id::BlockId;
pub use coins::*;

pub struct LoadBlocksArgs<'a> {
    path: &'a str,
    index_dir_path: &'a str,
    from_height: Option<u64>,
    network: &'a str,
    reorg_len: u64,
}

pub struct LoadBlocks {
    storage: ChainStorage,
    from_height: u64,
    reorg_len: u64,
}

impl LoadBlocks {
    pub fn new(data: LoadBlocksArgs<'_>) -> Self {
        let from_height = data.from_height.unwrap_or_default();

        Self {
            storage: ChainStorage::new(&ChainOptions {
                blockchain_dir: PathBuf::from_str(data.path).unwrap(),
                range: BlockHeightRange::new(from_height, None).unwrap(),
                coin: CoinType::from_str(data.network).expect("Unsupported network"),
                index_dir_path: PathBuf::from_str(data.index_dir_path).unwrap(),
            })
            .unwrap(),
            from_height,
            reorg_len: data.reorg_len,
        }
    }

    pub fn load_blocks(
        &mut self,
    ) -> impl Iterator<Item = blockchain::proto::block::Block> + use<'_> {
        let max_height = self.storage.max_height() - self.reorg_len;

        (self.from_height..=max_height).map(|x| self.storage.get_block(x).unwrap().unwrap())
    }

    pub fn get_block(&mut self, height: u64) -> Option<blockchain::proto::block::Block> {
        self.storage.get_block(height).unwrap()
    }
}
