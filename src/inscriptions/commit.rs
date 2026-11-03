use super::*;
use crate::inscriptions::indexer::DataToWrite;

pub(crate) fn commit_block(server: &Server, cache: &mut ReorgCache, height: u32, data: DataToWrite, handle_reorgs: bool) -> anyhow::Result<()> {
    // Caller owns the reorg-cache mutex, always before the publication gate.
    let _gate = server.sync_gate.write();
    let mut batch = rocksdb_wrapper::RocksBatch::default();
    if handle_reorgs {
        cache.new_block(height);
    }
    for processed in data.processed {
        processed.write(server, handle_reorgs.then_some(&mut *cache), &mut batch);
    }
    crate::sync::storage::append(
        &server.db,
        &mut batch,
        height,
        data.sync_balances,
        handle_reorgs && server.sync_config.enabled(),
        server.sync_config.retention,
    )?;
    batch.commit(&server.db.storage)?;
    server.catching_up.store(!handle_reorgs, std::sync::atomic::Ordering::Release);
    drop(_gate);
    for event in data.block_events {
        server.event_sender.send(event).ok();
    }
    if server.raw_event_sender.send(data.history).is_err() && !server.token.is_cancelled() {
        anyhow::bail!("Failed to send raw event");
    }
    Ok(())
}

pub(crate) fn mark_synced(server: &Server, tip: nint_blk::BlockId) -> anyhow::Result<()> {
    let _gate = server.sync_gate.write();
    if server.db.last_block.try_get(())? == Some(tip.height as u32)
        && server
            .db
            .block_info
            .try_get(tip.height as u32)?
            .is_some_and(|block| block.hash.to_string() == tip.hash.to_string())
    {
        server.catching_up.store(false, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}
