use super::*;

mod blk_file;
mod chain;
mod index;
mod reader;

pub use chain::ChainStorage;
pub use reader::BlockchainRead;

pub struct ChainOptions {
    pub blockchain_dir: PathBuf,
    pub range: crate::utils::BlockHeightRange,
    pub coin: CoinType,
    pub index_dir_path: PathBuf,
}

impl ChainOptions {
    pub fn new(path: &str, index_dir_path: &str, coin: CoinType, last_height: u32) -> Self {
        let dir = PathBuf::from_str(path).expect("Invalid path");
        let index_dir_path = PathBuf::from_str(index_dir_path).expect("Invalid INDEX_DIR path");
        let range = crate::utils::BlockHeightRange::new(last_height as u64, None).unwrap();

        Self {
            blockchain_dir: dir,
            coin,
            range,
            index_dir_path,
        }
    }
}
