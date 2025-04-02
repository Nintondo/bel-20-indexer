use super::*;

mod logging;
mod progress;

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
