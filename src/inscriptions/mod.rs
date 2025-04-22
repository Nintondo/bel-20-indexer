use super::*;
use crate::{
    server::threads::block_loader::{BlockBlkLoader, BlockRpcLoader},
    utils::address_encoder::{Decoder, Encoder},
};
use dutils::async_thread::{Thread, ThreadController};
use kanal::bounded;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod media;
mod parser;
mod searcher;
mod structs;
mod tag;
mod utils;

use envelope::{ParsedEnvelope, RawEnvelope};
use parser::InitialIndexer;
use searcher::InscriptionSearcher;
use structs::{Inscription, ParsedInscription};
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

pub async fn main_loop(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));
    let tip_hash = server.client.best_block_hash().await?;
    let tip_height = server.client.get_block_info(&tip_hash).await?.height as u32;

    let last_block = server.db.last_block.get(()).map(|x| x + 1).unwrap_or(1);

    warn!("Blocks to sync: {}", tip_height - last_block);

    {
        let (block_tx, block_rx) = bounded(50);

        let blk_loader = Arc::new(parking_lot::Mutex::new(BlockBlkLoader {
            magic: load_magic(),
            blk_dir: PathBuf::from_str(&BLK_DIR)?,
            from_block: Some(last_block),
            to_block: Some(tip_height - reorg::REORG_CACHE_MAX_LEN as u32),
        }));
        BlockBlkLoader::run(blk_loader.clone(), block_tx);

        let progress = crate::utils::Progress::begin("Indexing", tip_height as _, last_block as _);
        loop {
            let Some(data) = token.run_fn(block_rx.as_async().recv()).await else {
                break;
            };

            let (height, block, _) = data?;

            if height > tip_height - reorg::REORG_CACHE_MAX_LEN as u32 {
                break;
            }

            blk_loader.lock().from_block = Some(height);
            InitialIndexer::handle(height, block, server.clone(), None)
                .await
                .track()?;

            progress.inc(1);

            if token.is_cancelled() {
                return Ok(());
            }
        }
    }

    new_fetcher(token, server.clone(), reorg_cache.clone())
        .await
        .track()
        .ok();

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}

async fn new_fetcher(
    token: WaitToken,
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<reorg::ReorgCache>>,
) -> anyhow::Result<()> {
    let (block_tx, block_rx) = bounded(50);

    let block_loader = BlockRpcLoader {
        server: server.clone(),
        tx: block_tx,
        last_sent_block: Arc::default(),
    };

    ThreadController::new(block_loader)
        .with_name("BlockRpcLoader")
        .with_cancellation(token.clone())
        .with_invoke_frq(Duration::from_millis(250))
        .with_restart(Duration::from_secs(5))
        .kill()
        .run();

    loop {
        let Some(data) = token.run_fn(block_rx.as_async().recv()).await else {
            break;
        };
        let (height, block, _) = data?;

        let current_block_height = server.db.last_block.get(()).unwrap_or(0);

        if height <= current_block_height {
            let prev_block_hash = server
                .db
                .block_hashes
                .get(height - 1)
                .expect("Prev block hash must exist");

            if prev_block_hash != block.header.prev_blockhash {
                panic!(
                    "Block loader prepared bad reorg: got height {} current height {}, but prev hash mismatch for height - 1. Got {} but expected {}",
                    height, current_block_height,prev_block_hash,block.header.prev_blockhash
                )
            }

            let reorg_counter = current_block_height - height;
            warn!("Reorg detected: {} blocks", reorg_counter);
            reorg_cache.lock().restore(&server, height)?;
            server
                .event_sender
                .send(ServerEvent::Reorg(reorg_counter, height))
                .ok();
        }

        InitialIndexer::handle(height, block, server.clone(), Some(reorg_cache.clone()))
            .await
            .track()?;
    }

    Ok(())
}
