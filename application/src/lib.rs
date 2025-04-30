use bellscoin::Network;
use bellscoin::hashes::{Hash, sha256};
use core_utils::{load_env, load_opt_env};
use lazy_static::lazy_static;
use std::str::FromStr;

pub mod token_cache;
pub mod inscriptions;


lazy_static! {
    pub static ref URL: String = load_env!("RPC_URL");
    pub static ref USER: String = load_env!("RPC_USER");
    pub static ref PASS: String = load_env!("RPC_PASS");
    pub static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Bellscoin);
    pub static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    pub static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
