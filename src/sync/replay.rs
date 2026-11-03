use super::{
    error::{internal, response},
    storage,
};
use crate::{rest::ApiResult, Server};
use axum::{http::StatusCode, Json};
use nintondo_holdings_protocol::{BlockPage, BlockRef};

pub fn blocks(server: &Server, height: u32, hash: &str, limit: usize) -> ApiResult<Json<BlockPage>> {
    let _gate = server.sync_gate.read();
    let status = super::status(server).map_err(internal)?;
    // A restarting source temporarily forgets up to 30 blocks. Wait before
    // comparing hashes or expiring the consumer checkpoint.
    if height > status.tip.height || status.catching_up {
        return Ok(Json(BlockPage { status, blocks: vec![] }));
    }
    let ancestor = server
        .db
        .block_info
        .try_get(height)
        .map_err(internal)?
        .ok_or_else(|| internal(anyhow::anyhow!("Canonical checkpoint metadata missing")))?;
    if ancestor.hash.to_string() != hash {
        return Err(response(StatusCode::CONFLICT, "Checkpoint hash changed"));
    }
    if height < status.tip.height && height.saturating_add(1) < status.replay_start {
        return Err(response(StatusCode::GONE, "Checkpoint predates replay retention; import a snapshot"));
    }
    let mut blocks = Vec::new();
    let mut bytes = 0;
    for offset in 1..=limit as u32 {
        let Some(next) = height.checked_add(offset).filter(|h| *h <= status.tip.height) else {
            break;
        };
        let block = storage::read(&server.db, next).map_err(internal)?;
        let size = serde_json::to_vec(&block).map_err(internal)?.len();
        if !blocks.is_empty() && bytes + size > super::config::REPLAY_PAGE_BYTES {
            break;
        }
        bytes += size;
        blocks.push(block);
    }
    Ok(Json(BlockPage { status, blocks }))
}

pub fn block(server: &Server, height: u32) -> ApiResult<Json<BlockRef>> {
    let _gate = server.sync_gate.read();
    let tip = server.db.last_block.try_get(()).map_err(internal)?.unwrap_or_default();
    if height > tip {
        return Err(response(StatusCode::SERVICE_UNAVAILABLE, "Source has not reached this block"));
    }
    let info = server
        .db
        .block_info
        .try_get(height)
        .map_err(internal)?
        .ok_or_else(|| response(StatusCode::NOT_FOUND, "Block not found"))?;
    Ok(Json(BlockRef {
        height,
        hash: info.hash.to_string(),
    }))
}
