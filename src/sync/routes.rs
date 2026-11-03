use super::{
    config::*,
    error::{self, internal, response},
};
use crate::{rest::ApiResult, Server};
use aide::axum::{
    routing::{delete_with, get_with, post_with},
    ApiRouter,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Json,
};
use nintondo_holdings_protocol::{BlockPage, BlockRef, SnapshotCreated, SnapshotPage, SyncStatus};
use serde::Deserialize;
use std::sync::Arc;

pub fn router(server: Arc<Server>) -> ApiRouter<Arc<Server>> {
    ApiRouter::new()
        .api_route(
            "/status",
            get_with(status, |op| {
                op.tag("sync")
                    .security_requirement("SyncBearer")
                    .description("Authenticated source identity, tip and replay boundary")
            }),
        )
        .api_route(
            "/blocks",
            get_with(blocks, |op| {
                op.tag("sync")
                    .security_requirement("SyncBearer")
                    .description("Replay after a checkpoint; empty page while source is behind")
            }),
        )
        .api_route(
            "/block/{height}",
            get_with(block, |op| {
                op.tag("sync")
                    .security_requirement("SyncBearer")
                    .description("Canonical block; 503 while source is below requested height")
            }),
        )
        .api_route(
            "/snapshot",
            post_with(create, |op| {
                op.tag("sync").security_requirement("SyncBearer").description("Create a consistent holdings snapshot")
            }),
        )
        .api_route(
            "/snapshot/{id}",
            get_with(page, |op| {
                op.tag("sync")
                    .security_requirement("SyncBearer")
                    .description("Read snapshot page; only expired sessions return 410")
            })
            .merge(delete_with(close, |op| {
                op.tag("sync").security_requirement("SyncBearer").description("Release a snapshot session")
            })),
        )
        .api_route(
            "/events",
            get_with(super::events::subscribe, |op| {
                op.tag("sync")
                    .security_requirement("SyncBearer")
                    .description("SSE notifications using the shared SyncEvent contract")
            }),
        )
        .layer(middleware::from_fn_with_state(server, authorize))
}

async fn authorize(State(server): State<Arc<Server>>, request: axum::extract::Request, next: Next) -> Response {
    if !server.sync_config.enabled() {
        return response(StatusCode::NOT_FOUND, "Sync API disabled").map(axum::body::Body::from);
    }
    let supplied = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !server.sync_config.authorized(supplied) {
        return response(StatusCode::UNAUTHORIZED, "Invalid sync credentials").map(axum::body::Body::from);
    }
    next.run(request).await
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PageArgs {
    cursor: Option<String>,
    #[serde(default = "page_default")]
    limit: usize,
}
fn page_default() -> usize {
    PAGE_DEFAULT
}

async fn create(State(server): State<Arc<Server>>) -> ApiResult<Json<SnapshotCreated>> {
    tokio::task::spawn_blocking(move || server.snapshots.create(&server))
        .await
        .map_err(internal)?
        .map(Json)
        .map_err(error::snapshot)
}

async fn page(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(args): Query<PageArgs>) -> ApiResult<Json<SnapshotPage>> {
    if !(1..=PAGE_MAX).contains(&args.limit) {
        return Err(response(StatusCode::BAD_REQUEST, "Invalid page limit"));
    }
    let after = super::snapshots::decode_cursor(args.cursor.as_deref()).map_err(|_| response(StatusCode::BAD_REQUEST, "Invalid snapshot cursor"))?;
    tokio::task::spawn_blocking(move || server.snapshots.page(&server.db, &id, after, args.limit))
        .await
        .map_err(internal)?
        .map(Json)
        .map_err(error::snapshot)
}

async fn close(State(server): State<Arc<Server>>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    tokio::task::spawn_blocking(move || server.snapshots.close(&id)).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn status(State(server): State<Arc<Server>>) -> ApiResult<Json<SyncStatus>> {
    tokio::task::spawn_blocking(move || {
        let _gate = server.sync_gate.read();
        super::status(&server)
    })
    .await
    .map_err(internal)?
    .map(Json)
    .map_err(internal)
}

#[derive(Deserialize, schemars::JsonSchema)]
struct BlocksArgs {
    after_height: u32,
    after_hash: String,
    #[serde(default = "block_default")]
    limit: usize,
}
fn block_default() -> usize {
    BLOCK_DEFAULT
}

async fn blocks(State(server): State<Arc<Server>>, Query(args): Query<BlocksArgs>) -> ApiResult<Json<BlockPage>> {
    if !(1..=BLOCK_MAX).contains(&args.limit) || args.after_hash.len() != 64 || hex::decode(&args.after_hash).is_err() {
        return Err(response(StatusCode::BAD_REQUEST, "Invalid limit or block hash"));
    }
    tokio::task::spawn_blocking(move || super::replay::blocks(&server, args.after_height, &args.after_hash, args.limit))
        .await
        .map_err(internal)?
}

async fn block(State(server): State<Arc<Server>>, Path(height): Path<u32>) -> ApiResult<Json<BlockRef>> {
    tokio::task::spawn_blocking(move || super::replay::block(&server, height)).await.map_err(internal)?
}
