use application::NETWORK;
use bellscoin::Network;
use core_utils::load_env;
use lazy_static::lazy_static;

pub mod address_encoder;
pub mod blk;
pub mod inscriptions;
pub mod reorg;
pub mod server;

lazy_static! {
    pub static ref BLK_DIR: String = load_env!("BLK_DIR");
    pub static ref BLOCKCHAIN: String = load_env!("BLOCKCHAIN").to_lowercase();
    pub static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize =
        match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
            (Network::Bellscoin, "bells") => 133_000,
            (_, "doge") => usize::MAX,
            _ => 0,
        };
    pub static ref START_HEIGHT: u32 = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 26_371,
        (Network::Bellscoin, "doge") => 4_609_723,
        (Network::Testnet, "doge") => 4_260_514,
        _ => 0,
    };
}
