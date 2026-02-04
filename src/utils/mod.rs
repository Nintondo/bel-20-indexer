use super::*;

mod address_fullhash;
mod fullhash;
mod logging;
mod progress;
mod redact;

pub use address_fullhash::{fullhash_to_address_str, AddressesFullHash};
pub use fullhash::{ComputeScriptHash, FullHash, IsOpReturnHash};
pub use logging::init_logger;
pub use progress::Progress;
pub use redact::RedactedStr;

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
