use bellscoin::hashes::{sha256, Hash};
use bellscoin::Network;
use core_utils::{load_env, load_opt_env};
use lazy_static::lazy_static;
use std::str::FromStr;

pub mod address_encoder;
pub mod blk;
pub mod client;
pub mod inscriptions;
pub mod reorg;
pub mod server;

lazy_static! {
    static ref BLK_DIR: String = load_env!("BLK_DIR");
    static ref URL: String = load_env!("RPC_URL");
    static ref USER: String = load_env!("RPC_USER");
    static ref PASS: String = load_env!("RPC_PASS");
    static ref BLOCKCHAIN: String = load_env!("BLOCKCHAIN").to_lowercase();
    static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Bellscoin);
    static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize =
        match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
            (Network::Bellscoin, "bells") => 133_000,
            (_, "doge") => usize::MAX,
            _ => 0,
        };
    static ref START_HEIGHT: u32 = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 26_371,
        (Network::Bellscoin, "doge") => 4_609_723,
        (Network::Testnet, "doge") => 4_260_514,
        _ => 0,
    };
    static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}
