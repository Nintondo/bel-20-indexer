use super::{
    config::*,
    error::{internal, response},
};
use crate::{rest::ApiResult, Server};
use aide::{
    generate::GenContext,
    openapi::{Operation, Response as ApiResponse},
    OperationOutput,
};
use axum::{
    body::{Body, Bytes},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::{
    io::{self, Write},
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_stream::Stream;

pub struct Limits {
    pub work: Arc<Semaphore>,
    pub streams: Arc<Semaphore>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            work: Arc::new(Semaphore::new(EXPORT_WORKERS)),
            streams: Arc::new(Semaphore::new(EVENT_CONNECTIONS)),
        }
    }
}

pub struct Encoded<T>(Vec<u8>, Arc<OwnedSemaphorePermit>, PhantomData<T>);
impl<T: schemars::JsonSchema> OperationOutput for Encoded<T> {
    type Inner = T;
    fn operation_response(ctx: &mut GenContext, operation: &mut Operation) -> Option<ApiResponse> {
        <Json<T> as OperationOutput>::operation_response(ctx, operation)
    }
}
struct Chunk {
    bytes: Option<Bytes>,
    _permit: Arc<OwnedSemaphorePermit>,
}
impl Stream for Chunk {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.bytes.take().map(Ok))
    }
}
impl<T> IntoResponse for Encoded<T> {
    fn into_response(self) -> Response {
        let mut response = Response::new(Body::from_stream(Chunk {
            bytes: Some(self.0.into()),
            _permit: self.1,
        }));
        response.headers_mut().insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        response
    }
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > REPLAY_PAGE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("Sync response exceeds byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub async fn run<T: Serialize + Send + 'static>(server: Arc<Server>, work: impl FnOnce(&Server) -> ApiResult<T> + Send + 'static) -> ApiResult<Encoded<T>> {
    let permit = Arc::new(
        server
            .sync_limits
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| response(StatusCode::TOO_MANY_REQUESTS, "Sync workers busy; retry later"))?,
    );
    tokio::task::spawn_blocking(move || {
        // The permit survives request cancellation until blocking work finishes,
        // and survives serialization until the response body is released.
        let value = work(&server)?;
        let mut bytes = Bounded(Vec::new());
        serde_json::to_writer(&mut bytes, &value).map_err(internal)?;
        Ok(Encoded(bytes.0, permit, PhantomData))
    })
    .await
    .map_err(internal)?
}
