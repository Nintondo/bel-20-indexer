use crate::utils::RedactedStr;
use crate::{Blockchain, Network};
use std::fmt;

#[derive(Clone, Debug)]
pub struct Config {
    pub blk_dir: Option<String>,
    pub rpc_url: String,
    pub rpc_user: String,
    pub rpc_pass: String,
    pub blockchain: Blockchain,
    pub index_dir: Option<String>,
    pub network: Network,
    pub jubilee_height: usize,
    pub start_height: u32,
    pub server_url: String,
    pub db_path: String,
}

impl Config {
    pub fn new() -> Self {
        Self {
            blk_dir: crate::BLK_DIR.clone(),
            rpc_url: crate::URL.clone(),
            rpc_user: crate::USER.clone(),
            rpc_pass: crate::PASS.clone(),
            blockchain: crate::BLOCKCHAIN.clone(),
            index_dir: crate::INDEX_DIR.clone(),
            network: *crate::NETWORK,
            jubilee_height: *crate::JUBILEE_HEIGHT,
            start_height: *crate::START_HEIGHT,
            server_url: crate::SERVER_URL.clone(),
            db_path: crate::DB_PATH.clone(),
        }
    }

    pub fn redacted(&self) -> RedactedConfig<'_> {
        RedactedConfig(self)
    }
}

pub struct RedactedConfig<'a>(&'a Config);

impl fmt::Debug for RedactedConfig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let config = self.0;
        f.debug_struct("Config")
            .field("blk_dir", &config.blk_dir)
            .field("rpc_url", &RedactedStr(&config.rpc_url))
            .field("rpc_user", &RedactedStr(&config.rpc_user))
            .field("rpc_pass", &RedactedStr(&config.rpc_pass))
            .field("blockchain", &config.blockchain)
            .field("index_dir", &config.index_dir)
            .field("network", &config.network)
            .field("jubilee_height", &config.jubilee_height)
            .field("start_height", &config.start_height)
            .field("server_url", &config.server_url)
            .field("db_path", &config.db_path)
            .finish()
    }
}
