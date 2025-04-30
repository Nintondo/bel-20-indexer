use application::NETWORK;
use bellscoin::{
    Network,
    hashes::{Hash, sha256},
};
use core_utils::load_opt_env;
use lazy_static::lazy_static;

pub mod inscriptions;
pub mod reorg;
pub mod server;

const MAINNET_START_HEIGHT: u32 = 26_371;

lazy_static! {
    static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize = if let Network::Bellscoin = *NETWORK
    {
        133_000
    } else {
        0
    };
    static ref START_HEIGHT: u32 = match *NETWORK {
        Network::Bellscoin => MAINNET_START_HEIGHT,
        _ => 0,
    };
    static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
