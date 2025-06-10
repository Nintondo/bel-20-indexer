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
}

impl ChainOptions {
    pub fn new(path: &str, coin: CoinType, last_height: u32) -> Self {
        let dir = PathBuf::from_str(path).expect("Invalid path");
        let range = crate::utils::BlockHeightRange::new(last_height as u64, None).unwrap();

        Self {
            blockchain_dir: dir,
            coin,
            range,
        }
    }
}
