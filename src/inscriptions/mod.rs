use super::*;
use crate::{
    server::threads::block_loader::{BlockBlkLoader, BlockRpcLoader},
    utils::address_encoder::{Decoder, Encoder},
};
use dutils::async_thread::{Thread, ThreadController};
use kanal::bounded;

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
use structs::Inscription;
use tag::Tag;

pub use structs::Location;

pub fn load_decoder() -> Box<dyn Decoder> {
    let encoder_network = (*NETWORK).into();
    let decoder: Box<dyn Decoder> = match (*BLOCKCHAIN).as_ref() {
        "bells" => Box::new(BellscoinDecoder::new(encoder_network)),
        "doge" => Box::new(DogecoinDecoder::new(encoder_network)),
        _ => unimplemented!("Got unsupported blockchain"),
    };

    decoder
}

pub fn load_magic() -> [u8; 4] {
    match (*BLOCKCHAIN).as_ref() {
        "bells" => match *NETWORK {
            Network::Bellscoin => bellscoin::network::constants::Magic::BELLSCOIN.to_bytes(),
            Network::Testnet => bellscoin::network::constants::Magic::TESTNET.to_bytes(),
            Network::Signet => bellscoin::network::constants::Magic::SIGNET.to_bytes(),
            Network::Regtest => bellscoin::network::constants::Magic::REGTEST.to_bytes(),
            _ => unimplemented!(),
        },
        "doge" => match *NETWORK {
            Network::Bellscoin => nintondo_dogecoin::network::constants::Magic::BITCOIN.to_bytes(),
            Network::Testnet => nintondo_dogecoin::network::constants::Magic::TESTNET.to_bytes(),
            Network::Signet => nintondo_dogecoin::network::constants::Magic::SIGNET.to_bytes(),
            Network::Regtest => nintondo_dogecoin::network::constants::Magic::REGTEST.to_bytes(),
            _ => unimplemented!(),
        },
        _ => unimplemented!("Got unsupported blockchain"),
    }
}

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
        self.initial_index().await?;
        self.new_fetcher().await.track().ok();

        self.reorg_cache
            .lock()
            .restore_all(&self.server)
            .track()
            .ok();

        self.server.db.flush_all();

        Ok(())
    }

    async fn initial_index(&self) -> anyhow::Result<()> {
        let tip_hash = self.server.client.best_block_hash().await?;
        let tip_height = self.server.client.get_block_info(&tip_hash).await?.height as u32;

        let last_block = self
            .server
            .db
            .last_block
            .get(())
            .map(|x| x + 1)
            .unwrap_or(1);

        warn!("Blocks to sync: {}", tip_height - last_block);

        let (block_tx, block_rx) = bounded(50);

        let blk_loader = Arc::new(parking_lot::Mutex::new(BlockBlkLoader {
            magic: load_magic(),
            blk_dir: PathBuf::from_str(&BLK_DIR)?,
            from_block: Some(last_block),
            to_block: Some(tip_height - reorg::REORG_CACHE_MAX_LEN as u32),
        }));
        BlockBlkLoader::run(blk_loader.clone(), block_tx);

        let indexer = InscriptionIndexer::new(self.server.clone(), None);

        let progress = crate::utils::Progress::begin("Indexing", tip_height as _, last_block as _);
        let mut prev_height: Option<u32> = None;
        loop {
            let Some(Ok(data)) = self.server.token.run_fn(block_rx.as_async().recv()).await else {
                break;
            };

            let (height, block, _) = data;

            if let Some(prev) = prev_height {
                if prev + 1 != height {
                    panic!("Expected {} height but got {}", prev + 1, height);
                }
            }

            prev_height = Some(height);

            if height > tip_height - reorg::REORG_CACHE_MAX_LEN as u32 {
                break;
            }

            blk_loader.lock().from_block = Some(height);

            indexer.handle(height, block).await.track()?;

            progress.inc(1);

            if self.server.token.is_cancelled() {
                return Ok(());
            }
        }

        Ok(())
    }

    async fn new_fetcher(&self) -> anyhow::Result<()> {
        let (block_tx, block_rx) = bounded(50);

        let block_loader = BlockRpcLoader {
            server: self.server.clone(),
            tx: block_tx,
            last_sent_block: Arc::default(),
        };

        ThreadController::new(block_loader)
            .with_name("BlockRpcLoader")
            .with_cancellation(self.server.token.clone())
            .with_invoke_frq(Duration::from_millis(250))
            .with_restart(Duration::from_secs(5))
            .kill()
            .run();

        let indexer = InscriptionIndexer::new(self.server.clone(), Some(self.reorg_cache.clone()));

        loop {
            let Some(Ok(data)) = self.server.token.run_fn(block_rx.as_async().recv()).await else {
                break;
            };
            let (height, block, _) = data;

            let current_block_height = self.server.db.last_block.get(()).unwrap_or(0);

            if height <= current_block_height {
                let prev_block_hash = self
                    .server
                    .db
                    .block_info
                    .get(height - 1)
                    .expect("Prev block hash must exist")
                    .hash;

                if prev_block_hash != block.header.prev_blockhash {
                    panic!(
                        "Block loader prepared bad reorg: got height {} current height {}, but prev hash mismatch for height - 1. Got {} but expected {}",
                        height, current_block_height,prev_block_hash,block.header.prev_blockhash
                    )
                }

                let reorg_counter = current_block_height - height;
                warn!("Reorg detected: {} blocks", reorg_counter);
                self.reorg_cache.lock().restore(&self.server, height)?;
                self.server
                    .event_sender
                    .send(ServerEvent::Reorg(reorg_counter, height))
                    .ok();
            }

            indexer.handle(height, block).await.track()?;
        }

        Ok(())
    }
}
