use core_utils::load_env;
use core_utils::load_opt_env;
use core_utils::MAINNET_START_HEIGHT;
use lazy_static::lazy_static;
use nintondo_dogecoin::hashes::{sha256, Hash};
use nintondo_dogecoin::Network;
use std::str::FromStr;

pub mod inscriptions;
pub mod server;
pub mod reorg;
pub mod token_cache;

lazy_static! {
    static ref URL: String = load_env!("RPC_URL");
    static ref USER: String = load_env!("RPC_USER");
    static ref PASS: String = load_env!("RPC_PASS");
    pub static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Dogecoin);
    static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize = if let Network::Dogecoin = *NETWORK
    {
        133_000
    } else {
        0
    };
    static ref START_HEIGHT: u32 = match *NETWORK {
        Network::Dogecoin => MAINNET_START_HEIGHT,
        _ => 0,
    };
    static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    pub static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
