use super::*;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod indexer;
mod leaked;
mod parser;
mod processe_data;
mod searcher;
pub mod structs;
mod tag;
mod utils;

use envelope::{ParsedEnvelope, RawEnvelope};
use indexer::InscriptionIndexer;
use nint_blk::BlockEvent;
use parser::Parser;
use processe_data::ProcessedData;
use structs::Inscription;
use tag::Tag;

pub use structs::Location;

pub struct Indexer {
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<ReorgCache>>,
}

impl Indexer {
    pub fn new(server: Arc<Server>) -> Self {
        Self {
            reorg_cache: Arc::new(parking_lot::Mutex::new(ReorgCache::new())),
            server,
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
        let res = self.index();

        self.reorg_cache.lock().restore_all(&self.server).track().ok();
        self.server.db.flush_all();

        res
    }

    fn index(&self) -> anyhow::Result<()> {
        let rx = self.server.indexer.clone().parse_blocks();

        let indexer = InscriptionIndexer::new(self.server.clone(), self.reorg_cache.clone());

        let mut progress: Option<Progress> = Some(Progress::begin("Indexing", self.server.indexer.last_block.height, self.server.indexer.last_block.height));

        let mut prev_height: Option<u64> = None;
        while !self.server.token.is_cancelled() {
            let data = match rx.try_recv() {
                Ok(Some(data)) => data,
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => break,
            };
            if let Some(progress) = progress.as_mut() {
                progress.update_len(data.tip.saturating_sub(REORG_CACHE_MAX_LEN as u64));
            }

            let BlockEvent { block, id, tip, reorg_len } = data;

            let handle_reorgs = id.height > tip - REORG_CACHE_MAX_LEN as u64;

            if handle_reorgs {
                progress.take();
            }

            {
                let mut cache = self.reorg_cache.lock();
                if !cache.blocks.is_empty() && !handle_reorgs {
                    cache.blocks.clear();
                }
            }

            if reorg_len > 0 {
                warn!("Reorg detected: {} blocks", reorg_len);
                let restore_height = prev_height.unwrap_or_default().saturating_sub(reorg_len as u64);

                self.reorg_cache.lock().restore(&self.server, restore_height as u32)?;
                self.server.event_sender.send(ServerEvent::Reorg(reorg_len as u32, id.height as u32)).ok();
            }

            if let Some(last_reorg_height) = self.reorg_cache.lock().blocks.last_key_value().map(|x| x.0) {
                if last_reorg_height + 1 != id.height as u32 {
                    anyhow::bail!("Wrong reorg cache tip height. Expected {}, got {}", last_reorg_height + 1, id.height as u32);
                }
            }

            indexer.handle(id.height as u32, block, handle_reorgs).track()?;

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
