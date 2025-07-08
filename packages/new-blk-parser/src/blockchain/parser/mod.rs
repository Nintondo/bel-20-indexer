use super::*;

mod blk_file;
mod chain;
mod index;
mod reader;

pub use chain::ChainStorage;
pub use reader::BlockchainRead;

pub struct ChainOptions {
    pub blockchain_dir: Option<PathBuf>,
    pub range: crate::utils::BlockHeightRange,
    pub coin: CoinType,
    pub index_dir_path: Option<PathBuf>,
}

impl ChainOptions {
    pub fn new(path: Option<&str>, index_dir_path: Option<&str>, coin: CoinType, last_height: u32) -> Self {
        let dir = path.map(|path| PathBuf::from_str(path).expect("Invalid path"));
        let index_dir_path = index_dir_path.map(|index_dir_path| PathBuf::from_str(index_dir_path).expect("Invalid INDEX_DIR path"));
        let range = crate::utils::BlockHeightRange::new(last_height as u64, None).unwrap();

        Self {
            blockchain_dir: dir,
            coin,
            range,
            index_dir_path,
        }
    }
}
