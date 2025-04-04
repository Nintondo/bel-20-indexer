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

pub async fn retry_on_error<T, F, Fut>(
    attempts: u64,
    timeout_secs: u64,
    cancel_token: &WaitToken,
    operation: F,
) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, electrs_client::ClientError>>,
{
    let mut attempt = 0;
    let sleep = Duration::from_secs(5);
    let timeout = Duration::from_secs(timeout_secs);

    while attempt < attempts {
        attempt += 1;

        let future = tokio::time::timeout(timeout, operation());
        let Some(result) = cancel_token.run_fn(future).await else {
            return Err(anyhow::anyhow!("Operation cancelled"));
        };

        let response = match result {
            Ok(response) => response,
            Err(_) => {
                error!("Operation timed out");
                if attempt >= attempts {
                    return Err(anyhow::anyhow!("Operation timed out"));
                }
                tokio::time::sleep(sleep).await;
                continue;
            }
        };

        match response {
            Ok(success) => {
                return Ok(success);
            }
            Err(electrs_client::ClientError::Json(e)) => {
                warn!("Got client recovery error {e}, trying again...");
                if attempt >= attempts {
                    return Err(anyhow::anyhow!(e));
                }
                tokio::time::sleep(sleep).await;
                continue;
            }
            Err(e) => {
                return Err(anyhow::anyhow!(e));
            }
        }
    }

    Err(anyhow::anyhow!("Maximum retry attempts reached"))
}
