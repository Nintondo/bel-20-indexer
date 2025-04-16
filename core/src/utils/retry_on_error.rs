use std::time::Duration;
use dutils::wait_token::WaitToken;
use tracing::warn;

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

        let error = match result {
            Ok(response) => match response {
                Ok(success) => {
                    return Ok(success);
                }
                Err(electrs_client::ClientError::Reqwest(e)) if e.is_request() => {
                    anyhow::anyhow!(e)
                }
                Err(electrs_client::ClientError::Json(e)) => {
                    anyhow::anyhow!(e)
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(e));
                }
            },
            Err(_) => {
                anyhow::anyhow!("Operation timed out")
            }
        };

        if attempt >= attempts {
            return Err(error);
        }

        warn!("Got client recovery error {error}, trying again...");
        tokio::time::sleep(sleep).await;
    }

    Err(anyhow::anyhow!("Maximum retry attempts reached"))
}
