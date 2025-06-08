use crate::utils::Progress;

use super::*;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod indexer;
mod leaked;
mod media;
mod parser;
mod processe_data;
mod searcher;
pub mod structs;
mod tag;
mod utils;

use envelope::{ParsedEnvelope, RawEnvelope};
use indexer::InscriptionIndexer;
use nint_blk::BlockEvent;
use structs::Inscription;
use tag::Tag;

pub use structs::Location;

pub struct Indexer {
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<reorg::ReorgCache>>,
}

impl Indexer {
    pub fn new(server: Arc<Server>) -> Self {
        Self {
            reorg_cache: Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new())),
            server,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        self.index().await?;

        self.reorg_cache
            .lock()
            .restore_all(&self.server)
            .track()
            .ok();

        self.server.db.flush_all();

        Ok(())
    }

    async fn index(&self) -> anyhow::Result<()> {
        let rx = self.server.indexer.clone().parse_blocks().to_async();

        let mut indexer = InscriptionIndexer::new(self.server.clone(), None);

        let mut progress: Option<Progress> = Some(Progress::begin(
            "Indexing",
            self.server.indexer.last_height as u64,
            self.server.indexer.last_height as u64,
        ));

        let mut prev_height: Option<u64> = None;
        loop {
            let Some(Ok(data)) = self.server.token.run_fn(rx.recv()).await else {
                break;
            };
            if let Some(progress) = progress.as_mut() {
                progress.update_len(data.tip.saturating_sub(reorg::REORG_CACHE_MAX_LEN as u64));
            }

            let BlockEvent {
                block,
                id,
                tip,
                reorg_len,
            } = data;

            if id.height > tip - reorg::REORG_CACHE_MAX_LEN as u64 && indexer.reorg_cache.is_none()
            {
                indexer.reorg_cache = Some(self.reorg_cache.clone());
                progress.take();
            }

            if reorg_len > 0 {
                warn!("Reorg detected: {} blocks", reorg_len);
                let restore_height = prev_height
                    .unwrap_or_default()
                    .saturating_sub(reorg_len as u64);

                self.reorg_cache
                    .lock()
                    .restore(&self.server, restore_height as u32)?;
                self.server
                    .event_sender
                    .send(ServerEvent::Reorg(reorg_len as u32, id.height as u32))
                    .ok();
            }

            indexer.handle(id.height as u32, block).await.track()?;
            prev_height = Some(id.height);

            if let Some(progress) = progress.as_ref() {
                progress.inc(1);
            }

            if self.server.token.is_cancelled() {
                return Ok(());
            }
        }

        Ok(())
    }
}
