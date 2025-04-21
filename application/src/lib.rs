use std::str::FromStr;
use lazy_static::lazy_static;
use nintondo_dogecoin::hashes::{sha256, Hash};
use nintondo_dogecoin::Network;
use core_utils::{load_env, load_opt_env, MAINNET_START_HEIGHT};

pub mod server;
pub mod token_cache;
pub mod reorg;

lazy_static! {
    pub static ref URL: String = load_env!("RPC_URL");
    pub static ref USER: String = load_env!("RPC_USER");
    pub static ref PASS: String = load_env!("RPC_PASS");
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
    pub static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    pub static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
