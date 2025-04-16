pub use logging::init_logger;
pub use progress::Progress;

pub mod logging;
pub mod progress;
pub mod retry_on_error;

#[macro_export]
macro_rules! load_env {
    ($var:expr) => {
        std::env::var($var).expect(&format!("Environment variable {} not found", $var))
    };
}

#[macro_export]
macro_rules! load_opt_env {
    ($var:expr) => {
        std::env::var($var).ok()
    };
}
