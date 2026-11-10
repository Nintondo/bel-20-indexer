use super::{
    error::{internal, response},
    storage,
};
use crate::{rest::ApiResult, Server};
use axum::{http::StatusCode, Json};
use nintondo_holdings_protocol::{BlockPage, BlockRef};

pub fn blocks(server: &Server, height: u32, hash: &str, limit: usize) -> ApiResult<BlockPage> {
    let (status, snapshot) = {
        let _gate = server.sync_gate.read();
        (super::status(server).map_err(internal)?, server.db.storage.snapshot())
    };
    // A restarting source temporarily forgets up to 30 blocks. Wait before
    // comparing hashes or expiring the consumer checkpoint.
    if height > status.tip.height || status.catching_up {
        return Ok(BlockPage { status, blocks: vec![] });
    }
    let ancestor = snapshot
        .get(&server.db.block_info, height)
        .map_err(internal)?
        .ok_or_else(|| internal(anyhow::anyhow!("Canonical checkpoint metadata missing")))?;
    if !ancestor.hash.to_string().eq_ignore_ascii_case(hash) {
        return Err(response(StatusCode::CONFLICT, "Checkpoint hash changed"));
    }
    if height < status.tip.height && height.saturating_add(1) < status.replay_start {
        return Err(response(StatusCode::GONE, "Checkpoint predates replay retention; import a snapshot"));
    }
    let mut blocks = Vec::new();
    let mut bytes = 4096usize;
    for offset in 1..=limit as u32 {
        let Some(next) = height.checked_add(offset).filter(|h| *h <= status.tip.height) else {
            break;
        };
        let block = storage::read_snapshot(&server.db, &snapshot, next).map_err(internal)?;
        // Conservative JSON upper bound (six bytes per escaped UTF-8 byte).
        // Serialize only the final page, once, on the blocking worker.
        let size = 512usize.saturating_add(
            block
                .balances
                .iter()
                .map(|b| 256usize.saturating_add(6 * (b.address.len() + b.account_id.len() + b.tick.len() + b.token_id.len() + b.available.len() + b.transferable.len())))
                .sum::<usize>(),
        );
        if bytes.saturating_add(size) > super::config::REPLAY_PAGE_BYTES {
            if blocks.is_empty() {
                return Err(response(StatusCode::GONE, "Block exceeds replay byte limit; import a snapshot"));
            }
            break;
        }
        bytes += size;
        blocks.push(block);
    }
    Ok(BlockPage { status, blocks })
}

pub fn block(server: &Server, height: u32) -> ApiResult<Json<BlockRef>> {
    let _gate = server.sync_gate.read();
    if server.catching_up.load(std::sync::atomic::Ordering::Acquire) {
        return Err(response(StatusCode::SERVICE_UNAVAILABLE, "Source canonical tip is not verified"));
    }
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
