use super::*;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod media;
mod parser;
mod searcher;
mod structs;
mod tag;
mod utils;

use envelope::{ParsedEnvelope, RawEnvelope};
use searcher::InscriptionSearcher;
use structs::{Inscription, ParsedInscription};
use tag::Tag;
pub use utils::ScriptToAddr;

pub use structs::Location;

pub async fn main_loop(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));

    let tip_hash = server.client.best_block_hash().await?;
    let tip_height = server.client.get_block_info(&tip_hash).await?.height as u32;

    let last_block = server.db.last_block.get(());
    let mut last_block = last_block.map(|x| x + 1).unwrap_or(1);

    warn!("Blocks to sync: {}", tip_height - last_block);

    {
        let progress = crate::utils::Progress::begin("Indexing", tip_height as _, last_block as _);

        while last_block < tip_height - reorg::REORG_CACHE_MAX_LEN as u32 && !token.is_cancelled() {
            parser::InitialIndexer::handle(last_block, server.clone(), None)
                .await
                .track()
                .ok();
            last_block += 1;
            progress.inc(1);
        }
    }

    if !token.is_cancelled() {
        new_fetcher(last_block - 1, token, server.clone(), reorg_cache.clone())
            .await
            .track()
            .ok();
    }

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}

async fn new_fetcher(
    last_block: u32,
    token: WaitToken,
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<reorg::ReorgCache>>,
) -> anyhow::Result<()> {
    let mut tip = server.client.get_block_hash(last_block).await?;

    let mut repeater = token.repeat_until_cancel(Duration::from_millis(50));

    while repeater.next().await {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Index new blocks
        let current_tip = server.client.best_block_hash().await?;

        if current_tip != tip {
            let last_height = server.client.get_block_info(&tip).await?.height;
            let mut current_height = last_height as u32 + 1;
            let mut next_hash = server.client.get_block_hash(current_height).await?;

            let mut reorg_counter = 0;

            loop {
                let local_prev_hash = server.db.block_hashes.get(current_height - 1).unwrap();
                let prev_block_hash = server
                    .client
                    .get_block_info(&next_hash)
                    .await?
                    .previousblockhash
                    .unwrap();

                if prev_block_hash != local_prev_hash {
                    reorg_counter += 1;
                    current_height -= 1;
                    next_hash = server.client.get_block_hash(current_height).await?;
                } else {
                    break;
                }
            }

            if reorg_counter > 0 {
                warn!("Reorg detected: {} blocks", reorg_counter);
                server
                    .event_sender
                    .send(ServerEvent::Reorg(reorg_counter, current_height))
                    .ok();
                reorg_cache.lock().restore(&server, current_height)?;
            }

            parser::InitialIndexer::handle(
                current_height,
                server.clone(),
                Some(reorg_cache.clone()),
            )
            .await?;

            tip = next_hash;
        }
    }

    Ok(())
}
