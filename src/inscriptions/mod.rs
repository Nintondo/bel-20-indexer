use super::*;
use std::default::Default;

pub mod parser;
mod structs;
pub mod types;
mod utils;

use dutils::async_thread::Thread;
use electrs_client::{BlockMeta, Update};
use types::{InscriptionsTokenHistory, TokenHistoryData};
pub use utils::ScriptToAddr;

pub use structs::Location;

pub async fn main_loop(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let client = Arc::new(
        electrs_client::Client::<TokenHistoryData>::new_from_cfg(server.client.clone())
            .await
            .inspect_err(|e| {
                dbg!(e);
            })?,
    );

    let last_electris_block = client.get_last_electrs_block_meta().await?;
    let last_indexed_block = server.db.last_block.get(()).unwrap_or_default();

    if let Some(block_number) = last_electris_block
        .height
        .checked_sub(reorg::REORG_CACHE_MAX_LEN as u32)
    {
        if block_number > last_indexed_block {
            let end_block = client.get_electrs_block_meta(block_number).await?;
            initial_indexer(token.clone(), server.clone(), client.clone(), end_block).await?;
        }
    }

    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));
    if !token.is_cancelled() {
        let indexer_block_number = server.db.last_block.get(()).unwrap_or_default();
        let indexer_block_meta = client
            .get_electrs_block_meta(indexer_block_number)
            .await
            .anyhow()?;
        let last_history_id = server.db.last_history_id.get(()).unwrap_or_default();
        // set mock reorg data for block to start indexer
        // it's safe because this mock data will be dropped
        reorg_cache
            .lock()
            .new_block(indexer_block_meta.into(), last_history_id);
    }

    indexer(
        token.clone(),
        server.clone(),
        client.clone(),
        reorg_cache.clone(),
    )
    .await?;

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}

async fn initial_indexer(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
    end: BlockMeta,
) -> anyhow::Result<()> {
    info!("Start Initial Indexer");

    let last_electris_block = client.get_last_electrs_block_meta().await?;
    let last_indexer_block_number = server.db.last_block.get(()).unwrap_or_default();

    let progress = crate::utils::Progress::begin(
        "Indexing",
        last_electris_block.height as _,
        last_indexer_block_number as _,
    );

    let last_indexer_block = client
        .get_electrs_block_meta(last_indexer_block_number)
        .await?;

    let blocks_storage = Arc::new(tokio::sync::Mutex::new(
        crate::server::threads::blocks_loader::LoadedBlocks {
            from_block_number: last_indexer_block.height,
            to_block_number: last_electris_block.height,
            ..Default::default()
        },
    ));

    let blocks_loader = dutils::async_thread::ThreadController::new(
        crate::server::threads::blocks_loader::BlocksLoader {
            storage: blocks_storage.clone(),
            client: client.clone(),
        },
    )
    .with_name("BlocksLoader")
    .with_restart(Duration::from_secs(5))
    .with_invoke_frq(Duration::from_secs(1))
    .with_cancellation(token.clone())
    .run();

    let mut sleep = token.repeat_until_cancel(Duration::from_secs(1));
    let mut is_reach_end = false;
    while !is_reach_end {
        let Some(blocks) = blocks_storage.lock().await.take_blocks() else {
            if !sleep.next().await || token.is_cancelled() {
                return Ok(());
            }
            continue;
        };

        let last_indexer_block_number = server.db.last_block.get(()).unwrap_or_default();
        let first_block_number = blocks
            .blocks
            .first()
            .map(|x| match x {
                electrs_client::Update::AddBlock { height, .. } => *height,
                _ => unimplemented!(),
            })
            .expect("Must exist");

        if last_indexer_block_number == 0 {
            progress.reset_c(first_block_number as _);
        }

        if last_indexer_block_number != 0 && last_indexer_block_number != first_block_number - 1 {
            panic!(
                "Got blocks with gap, in db #{last_indexer_block_number} but got #{first_block_number}"
            );
        }

        let mut updates = Vec::<types::ParsedTokenHistoryData>::new();

        for block in blocks.blocks {
            match block {
                electrs_client::Update::AddBlock { block, .. } => {
                    let casted_block: types::ParsedTokenHistoryData = block
                        .try_into()
                        .inspect_err(|e| {
                            dbg!(e);
                        })
                        .anyhow()?;

                    if casted_block.block_info.height == end.height {
                        is_reach_end = true;
                        break;
                    }

                    updates.push(casted_block);
                }

                _ => unreachable!(),
            }
        }

        let blocks_counter = updates.len();

        let now = Instant::now();
        parser::InitialIndexer::handle_batch(updates, &server, None).await;

        info!(
            "handle_batch #{} took {}s",
            server.db.last_block.get(()).unwrap_or_default(),
            now.elapsed().as_secs_f32()
        );

        progress.inc(blocks_counter as _);
    }

    blocks_loader.abort();

    Ok(())
}

async fn indexer(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
    reorg_cache: Arc<parking_lot::Mutex<reorg::ReorgCache>>,
) -> anyhow::Result<()> {
    info!("Start Indexer");

    let mut repeater = token.repeat_until_cancel(Duration::from_secs(3));
    while repeater.next().await {
        let mut last_index_height = server.db.last_block.get(()).unwrap_or_default();
        let last_indexer_block = client.get_electrs_block_meta(last_index_height).await?;
        let last_electris_block = client.get_last_electrs_block_meta().await?;

        if let Some(blocks_gap) = last_electris_block
            .height
            .checked_sub(last_indexer_block.height)
        {
            if blocks_gap == 0 && last_electris_block.block_hash == last_indexer_block.block_hash {
                info!("Indexer has the same block, sleep for a while ...");
                continue;
            } else {
                info!(
                    "Indexer has {}, elects has {}",
                    last_index_height, last_electris_block.height
                );
            }
        } else {
            warn!(
                "Indexer has block number {} but got {}, sleep for a while ...",
                last_indexer_block.height, last_electris_block.height
            );
            continue;
        };

        let blocks = reorg_cache.lock().get_blocks_headers();
        let Some(updates) = token.run_fn(load_blocks(&client, &blocks)).await else {
            break;
        };

        let updates = updates?;
        if updates.is_empty() {
            info!("Got empty updates, sleep for a while ...");
            continue;
        }

        let mut parsed_updates = Vec::<types::ParsedTokenHistoryData>::new();

        for block in updates {
            match block {
                electrs_client::Update::AddBlock { block, height, .. } => {
                    let casted_block: types::ParsedTokenHistoryData = block
                        .try_into()
                        .inspect_err(|e| {
                            dbg!(e);
                        })
                        .anyhow()?;

                    parsed_updates.push(casted_block);
                    last_index_height = height;
                }
                Update::RemoveBlock { height } | Update::RemoveCachedBlock { height, .. } => {
                    let reorg_counter = last_index_height - height;

                    warn!(
                        "Reorg detected: {} blocks, reorg height {}",
                        reorg_counter, height
                    );

                    reorg_cache
                        .lock()
                        .restore(&server, height)
                        .inspect_err(|e| {
                            dbg!(e);
                        })?;

                    server
                        .event_sender
                        .send(ServerEvent::Reorg(reorg_counter, height))
                        .ok();
                    last_index_height -= reorg_counter;
                }
            }
        }

        parser::InitialIndexer::handle_batch(parsed_updates, &server, Some(reorg_cache.clone()))
            .await;
    }
    Ok(())
}

async fn load_blocks(
    client: &electrs_client::Client<TokenHistoryData>,
    from: &[BlockHeader],
) -> anyhow::Result<Vec<electrs_client::Update<TokenHistoryData>>> {
    let from: Vec<_> = from.iter().map(|f| f.into()).collect();
    let updates = client
        .fetch_updates::<InscriptionsTokenHistory>(&from)
        .await
        .inspect_err(|e| {
            dbg!(e);
        })?;

    let (new_blocks, reorgs) = updates.iter().fold((0, 0), |(inserts, reorgs), v| match v {
        electrs_client::Update::AddBlock { .. } => (inserts + 1, reorgs),
        electrs_client::Update::RemoveBlock { .. } => (inserts, reorgs + 1),
        electrs_client::Update::RemoveCachedBlock { .. } => (inserts, reorgs + 1),
    });

    info!("Applying new blocks reorgs: {reorgs} new_blocks: {new_blocks}");

    Ok(updates)
}
