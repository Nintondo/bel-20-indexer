use std::str::FromStr;
use lazy_static::lazy_static;
use bellscoin::hashes::{sha256, Hash};
use bellscoin::Network;
use core_utils::{load_env, load_opt_env, MAINNET_START_HEIGHT};

pub mod token_cache;

lazy_static! {
    pub static ref URL: String = load_env!("RPC_URL");
    pub static ref USER: String = load_env!("RPC_USER");
    pub static ref PASS: String = load_env!("RPC_PASS");
    pub static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Bellscoin);
    pub static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize = if let Network::Bellscoin = *NETWORK
    {
        133_000
    } else {
        0
    };
    pub static ref START_HEIGHT: u32 = match *NETWORK {
        Network::Bellscoin => MAINNET_START_HEIGHT,
        _ => 0,
    };
    pub static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    pub static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
