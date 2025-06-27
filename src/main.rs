extern crate serde;
#[macro_use]
extern crate tracing;

use {
    crate::server::threads::EventSender,
    axum::{
        extract::{Path, Query, State},
        http::Response,
        response::IntoResponse,
        routing::get,
        Json, Router,
    },
    bellscoin::{
        hashes::{sha256, Hash},
        opcodes, script, BlockHash, Network, OutPoint, TxOut, Txid,
    },
    db::*,
    dutils::{
        error::{ApiError, ContextWrapper},
        wait_token::WaitToken,
    },
    inscriptions::{Indexer, Location},
    itertools::Itertools,
    num_traits::{FromPrimitive, Zero},
    reorg::{ReorgCache, REORG_CACHE_MAX_LEN},
    rocksdb_wrapper::{RocksDB, RocksTable, UsingConsensus, UsingSerde},
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    serde_with::{serde_as, DisplayFromStr},
    server::{Server, ServerEvent},
    std::{
        borrow::Cow,
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        fmt::{Display, Formatter},
        future::IntoFuture,
        iter::Peekable,
        ops::{Deref, RangeInclusive},
        str::FromStr,
        sync::{atomic::AtomicU64, Arc},
        time::{Duration, Instant},
    },
    tokens::*,
    tracing::info,
    tracing_indicatif::span_ext::IndicatifSpanExt,
    utils::*,
};

mod inscriptions;
mod reorg;
mod rest;
mod tokens;
#[macro_use]
mod utils;
mod db;
mod server;

pub type Fixed128 = nintypes::utils::fixed::Fixed128<18>;
const OP_RETURN_ADDRESS: &str = "BURNED";
const NON_STANDARD_ADDRESS: &str = "non-standard";

define_static! {
    OP_RETURN_HASH: FullHash = OP_RETURN_ADDRESS.compute_script_hash();
    BLK_DIR: String = load_env!("BLK_DIR");
    URL: String = load_env!("RPC_URL");
    USER: String = load_env!("RPC_USER");
    PASS: String = load_env!("RPC_PASS");
    BLOCKCHAIN: String = load_env!("BLOCKCHAIN").to_lowercase();
    INDEX_DIR: String = load_env!("INDEX_DIR");
    NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Bellscoin);
    // multiple input inscription scan activation
    JUBILEE_HEIGHT: usize = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 133_000,
        (_, "doge") => usize::MAX,
        _ => 0,
    };
    // first token block height
    START_HEIGHT: u32 = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 26_371,
        (Network::Bellscoin, "doge") => 4_609_001,
        (Network::Testnet, "doge") => 4_260_001,
        _ => 0,
    };
    SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
    DB_PATH: String = load_opt_env!("DB_PATH").unwrap_or("rocksdb".to_string());
}

fn main() {
    dotenv::dotenv().ok();
    utils::init_logger();

    dbg!(&*BLK_DIR, &*URL, &*USER, &*PASS, &*BLOCKCHAIN, *NETWORK, *JUBILEE_HEIGHT, *START_HEIGHT, &*SERVER_URL,);

    let (raw_event_tx, event_tx, server) = Server::new(&DB_PATH).unwrap();

    let server = Arc::new(server);

    shutdown_handler(server.token.clone());

    let rest_server = server.clone();
    std::thread::spawn(move || {
        let rest_runtime = spawn_runtime("rest".to_string(), Some(20));
        rest_runtime.block_on(run_rest(rest_server))
    });

    let event_sender = EventSender {
        event_tx,
        raw_event_tx,
        server: server.clone(),
    };

    let event_sender = std::thread::spawn(move || event_sender.run());

    let main_result = Indexer::new(server.clone()).run();
    server.token.cancel();

    info!("Server is finished");

    let event_sender_result = event_sender.join().unwrap();

    main_result.track().ok();
    event_sender_result.track().ok();
}

async fn run_rest(server: Arc<Server>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&*SERVER_URL).await.unwrap();

    let rest = axum::serve(listener, rest::get_router(server.clone()))
        .with_graceful_shutdown(server.token.cancelled())
        .into_future();

    let deadline = async move {
        server.token.cancelled().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    };

    tokio::select! {
        v = rest => {
            info!("Rest finished");
            v.anyhow()
        }
        _ = deadline => {
            warn!("Rest server shutdown timeout");
            Ok(())
        }
    }
}

fn spawn_runtime(name: String, priority: Option<u8>) -> tokio::runtime::Runtime {
    if let Some(priority) = priority {
        if let Err(e) = thread_priority::set_current_thread_priority(priority.try_into().unwrap()) {
            warn!("can't set priority {priority:?}, error {e:?}");
        };
    }

    let runtime = tokio::runtime::Builder::new_multi_thread().thread_name(&name).enable_all().build().unwrap();

    runtime
}

fn shutdown_handler(token: dutils::wait_token::WaitToken) {
    let _: std::thread::JoinHandle<Result<(), std::io::Error>> = std::thread::spawn(move || {
        let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT]).inspect_err(|_| {
            token.cancel();
        })?;

        for _ in &mut signals {
            token.cancel();
        }

        Ok(())
    });
}
