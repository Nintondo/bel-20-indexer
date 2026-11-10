use super::*;
use crate::inscriptions::indexer::DataToWrite;

pub(crate) fn commit_block(server: &Server, cache: &mut ReorgCache, height: u32, data: DataToWrite, handle_reorgs: bool) -> anyhow::Result<()> {
    // Caller owns the reorg-cache mutex, always before the publication gate.
    let _gate = server.sync_gate.write();
    let mut batch = rocksdb_wrapper::RocksBatch::default();
    let tip = data.processed.iter().find_map(|entry| match entry {
        super::process_data::ProcessedData::Info { block_info, .. } => Some((height, block_info.hash.to_byte_array())),
        _ => None,
    });
    let mut pending = ReorgCache::new();
    pending.new_block(height);
    for processed in data.processed {
        processed.write(server, Some(&mut pending), &mut batch);
    }
    let start = match crate::sync::storage::append(
        &server.db,
        &mut batch,
        height,
        data.sync_balances,
        handle_reorgs && server.sync_config.enabled(),
        server.sync_config.retention,
    ) {
        Ok(start) => start,
        Err(error) => {
            error!("Export failed at {height}; publishing replay gap: {error:#}");
            let floor = server.sync_boundary.load(std::sync::atomic::Ordering::Acquire);
            crate::sync::storage::invalidate(&server.db, &mut batch, height, floor);
            floor.max(height.saturating_add(1))
        }
    };
    batch.set(&server.db.sync_state, (), crate::sync::storage::ReplayState::V2 { start, tip });
    batch.commit(&server.db.storage)?;
    server.sync_boundary.fetch_max(start, std::sync::atomic::Ordering::AcqRel);
    cache.new_block(height);
    cache.blocks.insert(height, pending.blocks.pop_last().unwrap().1);
    server.catching_up.store(true, std::sync::atomic::Ordering::Release);
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
    let matches = (|| -> anyhow::Result<bool> {
        Ok(server.db.last_block.try_get(())? == Some(tip.height as u32)
            && server
                .db
                .block_info
                .try_get(tip.height as u32)?
                .is_some_and(|block| block.hash.to_string() == tip.hash.to_string()))
    })();
    match matches {
        Ok(true) => server.catching_up.store(false, std::sync::atomic::Ordering::Release),
        Ok(false) => {}
        Err(error) => {
            // Fail closed for exports; a transient read error must not stop indexing.
            server.catching_up.store(true, std::sync::atomic::Ordering::Release);
            error!("Cannot verify export readiness: {error:#}");
        }
    }
    Ok(())
}
