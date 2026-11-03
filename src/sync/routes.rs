use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use nintondo_holdings_protocol::{BlockPage, BlockRef};
use serde::Deserialize;
use std::sync::Arc;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

pub fn router() -> Router<Arc<Server>> {
    Router::new()
        .route("/status", get(status))
        .route("/events", get(super::events::subscribe))
        .route("/blocks", get(blocks))
        .route("/block/{height}", get(block))
        .route("/snapshot", post(create))
        .route("/snapshot/{id}", get(page).delete(close))
}

fn unavailable(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::SERVICE_UNAVAILABLE, e.to_string())
}

#[derive(Deserialize)]
struct PageArgs {
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    500
}

async fn create(State(server): State<Arc<Server>>) -> ApiResult<nintondo_holdings_protocol::SnapshotCreated> {
    tokio::task::spawn_blocking(move || server.snapshots.create(&server))
        .await
        .map_err(unavailable)?
        .map(Json)
        .map_err(unavailable)
}

async fn page(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(args): Query<PageArgs>) -> ApiResult<nintondo_holdings_protocol::SnapshotPage> {
    if !(1..=1000).contains(&args.limit) {
        return Err((StatusCode::BAD_REQUEST, "limit must be 1..1000".into()));
    }
    if args.cursor.as_ref().is_some_and(|c| c.len() != 72 || hex::decode(c).is_err()) {
        return Err((StatusCode::BAD_REQUEST, "Invalid snapshot cursor".into()));
    }
    tokio::task::spawn_blocking(move || server.snapshots.page(&server.db, &id, args.cursor, args.limit))
        .await
        .map_err(unavailable)?
        .map(Json)
        .map_err(|e| (StatusCode::GONE, e.to_string()))
}

async fn close(State(server): State<Arc<Server>>, Path(id): Path<String>) -> StatusCode {
    server.snapshots.close(&id);
    StatusCode::NO_CONTENT
}

async fn status(State(server): State<Arc<Server>>) -> ApiResult<nintondo_holdings_protocol::SyncStatus> {
    tokio::task::spawn_blocking(move || {
        let _gate = server.sync_gate.read();
        super::status(&server.db)
    })
    .await
    .map_err(unavailable)?
    .map(Json)
    .map_err(unavailable)
}

#[derive(Deserialize)]
struct BlocksArgs {
    after_height: u32,
    after_hash: String,
    #[serde(default = "block_limit")]
    limit: usize,
}
fn block_limit() -> usize {
    50
}

async fn blocks(State(server): State<Arc<Server>>, Query(args): Query<BlocksArgs>) -> ApiResult<BlockPage> {
    if !(1..=100).contains(&args.limit) || args.after_hash.len() != 64 || hex::decode(&args.after_hash).is_err() {
        return Err((StatusCode::BAD_REQUEST, "Invalid limit or block hash".into()));
    }
    tokio::task::spawn_blocking(move || {
        let _gate = server.sync_gate.read();
        let status = super::status(&server.db).map_err(unavailable)?;
        let ancestor = server.db.block_info.get(args.after_height);
        if ancestor.is_none_or(|b| b.hash.to_string() != args.after_hash) || args.after_height > status.tip.height {
            return Err((StatusCode::CONFLICT, "Checkpoint is no longer canonical".into()));
        }
        let mut blocks = Vec::new();
        for offset in 1..=args.limit as u32 {
            let Some(height) = args.after_height.checked_add(offset).filter(|h| *h <= status.tip.height) else {
                break;
            };
            blocks.push(
                server
                    .db
                    .sync_blocks
                    .get(height)
                    .ok_or((StatusCode::GONE, "Block predates replay log; create a new snapshot".into()))?,
            );
        }
        Ok(Json(BlockPage { status, blocks }))
    })
    .await
    .map_err(unavailable)?
}

async fn block(State(server): State<Arc<Server>>, Path(height): Path<u32>) -> ApiResult<BlockRef> {
    tokio::task::spawn_blocking(move || {
        let _gate = server.sync_gate.read();
        let tip = server.db.last_block.get(()).unwrap_or_default();
        let info = server
            .db
            .block_info
            .get(height)
            .filter(|_| height <= tip)
            .ok_or((StatusCode::NOT_FOUND, "Block not found".into()))?;
        Ok(Json(BlockRef {
            height,
            hash: info.hash.to_string(),
        }))
    })
    .await
    .map_err(unavailable)?
}
