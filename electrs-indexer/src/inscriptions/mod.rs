use dutils::async_thread::Thread;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod parser;
mod utils;

use crate::reorg;
use crate::server::Server;
use core_utils::interfaces::server::{
    DBPort, EventSenderPort,
    TokenPort,
};
use core_utils::types::loaded_blocks::LoadedBlocks;
use core_utils::types::server::ServerEvent;
use core_utils::types::structs::BlockHeader;
use core_utils::types::token_history::{
    InscriptionsTokenHistory, ParsedTokenHistoryData, TokenHistoryData,
};
use core_utils::utils::Progress;
use core_utils::utils::retry_on_error::retry_on_error;
use dutils::error::ContextWrapper;
use dutils::wait_token::WaitToken;
use electrs_client::{BlockMeta, Update};
use tracing::{info, warn};
pub use utils::ScriptToAddr;

pub async fn main_loop(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
) -> anyhow::Result<()> {
    let last_electrs_block =
        retry_on_error(30, 20, &token, || client.get_last_electrs_block_meta()).await?;

    let last_indexed_block = server.get_db().last_block.get(()).unwrap_or_default();

    if let Some(block_number) = last_electrs_block
        .height
        .checked_sub(reorg::REORG_CACHE_MAX_LEN as u32)
    {
        if block_number > last_indexed_block {
            let end_block = retry_on_error(30, 20, &token, || {
                client.get_electrs_block_meta(block_number)
            })
            .await?;

            initial_indexer(token.clone(), server.clone(), client.clone(), end_block).await?;
        }
    }

    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));
    if !token.is_cancelled() {
        let indexer_block_number = server.get_db().last_block.get(()).unwrap_or_default();
        let indexer_block_meta = retry_on_error(30, 20, &token, || {
            client.get_electrs_block_meta(indexer_block_number)
        })
        .await?;

        let last_history_id = server.get_db().last_history_id.get(()).unwrap_or_default();
        // set mock reorg data for block to start indexer
        // it's safe because this mock data will be dropped
        reorg_cache
            .lock()
            .new_block(indexer_block_meta.into(), last_history_id);
    }

    let indexer_result = indexer(
        token.clone(),
        server.clone(),
        client.clone(),
        reorg_cache.clone(),
    )
    .await;

    info!("Server is finished");

    reorg_cache.lock().restore_all(server.as_ref()).track().ok();

    server.get_db().flush_all();

    indexer_result
}

async fn initial_indexer(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
    end: BlockMeta,
) -> anyhow::Result<()> {
    info!("Start Initial Indexer");

    let last_electrs_block = client.get_last_electrs_block_meta().await?;
    let last_indexer_block_number = server.get_db().last_block.get(()).unwrap_or_default();

    let progress = Progress::begin(
        "Indexing",
        last_electrs_block.height as _,
        last_indexer_block_number as _,
    );

    let last_indexer_block = client
        .get_electrs_block_meta(last_indexer_block_number)
        .await?;

    let blocks_storage = Arc::new(tokio::sync::Mutex::new(LoadedBlocks {
        from_block_number: last_indexer_block.height,
        to_block_number: last_electrs_block.height,
        ..Default::default()
    }));

    let blocks_loader = dutils::async_thread::ThreadController::new(
        crate::server::threads::blocks_loader::BlocksLoader {
            storage: blocks_storage.clone(),
            client: client.clone(),
        },
    )
    .with_name("BlocksLoader")
    .with_restart(Duration::from_secs(5))
    .with_invoke_frq(Duration::from_millis(100))
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

        let last_indexer_block_number = server.get_db().last_block.get(()).unwrap_or_default();
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

        let mut updates = Vec::<ParsedTokenHistoryData>::new();

        for block in blocks.blocks {
            match block {
                electrs_client::Update::AddBlock { block, .. } => {
                    let casted_block: ParsedTokenHistoryData = block
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
        parser::InitialIndexer::handle_batch(updates, server.as_ref(), None).await;

        info!(
            "handle_batch #{} took {}s",
            server.get_db().last_block.get(()).unwrap_or_default(),
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
        let last_index_height = server.get_db().last_block.get(()).unwrap_or_default();

        let last_indexer_block = retry_on_error(u64::MAX, 20, &token, || {
            client.get_electrs_block_meta(last_index_height)
        })
        .await?;

        let last_electrs_block = retry_on_error(u64::MAX, 20, &token, || {
            client.get_last_electrs_block_meta()
        })
        .await?;

        if let Some(blocks_gap) = last_electrs_block
            .height
            .checked_sub(last_indexer_block.height)
        {
            if blocks_gap == 0 && last_electrs_block.block_hash == last_indexer_block.block_hash {
                info!("Indexer has the same block, sleep for a while ...");
                continue;
            } else {
                info!(
                    "Indexer has {}, electrs has {}",
                    last_index_height, last_electrs_block.height
                );
            }
        } else {
            warn!(
                "Indexer has block number {} but got {}, sleep for a while ...",
                last_indexer_block.height, last_electrs_block.height
            );
            continue;
        };

        let blocks = reorg_cache.lock().get_blocks_headers();

        let updates = load_blocks(&server.get_token(), &client, &blocks).await?;

        if updates.is_empty() {
            info!("Got empty updates, sleep for a while ...");
            continue;
        }

        for block in updates {
            match block {
                electrs_client::Update::AddBlock { block, .. } => {
                    let casted_block: ParsedTokenHistoryData = block
                        .try_into()
                        .inspect_err(|e| {
                            dbg!(e);
                        })
                        .anyhow()?;

                    parser::InitialIndexer::handle_batch(
                        vec![casted_block],
                        server.as_ref(),
                        Some(reorg_cache.clone()),
                    )
                    .await;
                }
                Update::RemoveBlock { height } | Update::RemoveCachedBlock { height, .. } => {
                    let last_index_height = server.get_db().last_block.get(()).unwrap_or_default();
                    let reorg_counter = last_index_height - height;

                    warn!(
                        "Reorg detected: {} blocks, reorg height {}",
                        reorg_counter, height
                    );

                    reorg_cache
                        .lock()
                        .restore(server.as_ref(), height)
                        .inspect_err(|e| {
                            dbg!(e);
                        })?;

                    server
                        .get_event_sender()
                        .send(ServerEvent::Reorg(reorg_counter, height))
                        .ok();
                }
            }
        }
    }
    Ok(())
}

async fn load_blocks(
    token: &WaitToken,
    client: &electrs_client::Client<TokenHistoryData>,
    from: &[BlockHeader],
) -> anyhow::Result<Vec<electrs_client::Update<TokenHistoryData>>> {
    let from: Vec<_> = from.iter().map(|f| f.into()).collect();

    let updates = retry_on_error(10, 60, token, || {
        client.fetch_updates::<InscriptionsTokenHistory>(&from)
    })
    .await?;

    if updates.is_empty() {
        return Ok(Vec::new());
    }

    let (new_blocks, reorgs) = updates.iter().fold((0, 0), |(inserts, reorgs), v| match v {
        electrs_client::Update::AddBlock { .. } => (inserts + 1, reorgs),
        electrs_client::Update::RemoveBlock { .. } => (inserts, reorgs + 1),
        electrs_client::Update::RemoveCachedBlock { .. } => (inserts, reorgs + 1),
    });

    info!("Applying new blocks reorgs: {reorgs} new_blocks: {new_blocks}");

    Ok(updates)
}
