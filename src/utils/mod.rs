use super::*;

pub mod address_encoder;
mod address_fullhash;
pub mod blk;
mod client;
mod logging;
mod progress;

pub use address_fullhash::AddressesFullHash;
pub use client::AsyncClient;
pub use logging::init_logger;
pub use progress::Progress;

macro_rules! load_env {
    ($var:expr) => {
        std::env::var($var).expect(&format!("Environment variable {} not found", $var))
    };
}

macro_rules! load_opt_env {
    ($var:expr) => {
        std::env::var($var).ok()
    };
}

macro_rules! define_static {
    ($($name:ident: $ty:ty = $value:expr);* ;) => {
        $(
            static $name: std::sync::LazyLock<$ty> = std::sync::LazyLock::new(|| $value);
        )*
    };
}
