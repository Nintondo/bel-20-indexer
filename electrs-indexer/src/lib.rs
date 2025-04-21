use core_utils::load_env;
use core_utils::load_opt_env;
use core_utils::MAINNET_START_HEIGHT;
use lazy_static::lazy_static;
use nintondo_dogecoin::hashes::{sha256, Hash};
use nintondo_dogecoin::Network;
use std::str::FromStr;

pub mod inscriptions;

