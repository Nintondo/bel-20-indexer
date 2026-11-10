use axum::{http::StatusCode, response::Response};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Snapshot expired or was closed")]
    Expired,
    #[error("Snapshot capacity reached")]
    Capacity,
    #[error("Indexer is catching up")]
    CatchingUp,
}

pub fn response(status: StatusCode, message: impl std::fmt::Display) -> Response<String> {
    Response::builder().status(status).body(message.to_string()).expect("valid response")
}

pub fn internal(error: impl Into<anyhow::Error>) -> Response<String> {
    let error = error.into();
    tracing::error!(error = %format!("{error:#}"), "Holdings sync request failed");
    response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

pub fn snapshot(error: anyhow::Error) -> Response<String> {
    match error.downcast_ref::<SyncError>() {
        Some(SyncError::Expired) => response(StatusCode::GONE, error),
        Some(SyncError::Capacity) => response(StatusCode::TOO_MANY_REQUESTS, error),
        Some(SyncError::CatchingUp) => response(StatusCode::SERVICE_UNAVAILABLE, error),
        None => internal(error),
    }
}
