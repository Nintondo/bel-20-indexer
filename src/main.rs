#[macro_use]
extern crate tracing;
extern crate serde;

use {
    axum::{
        body::Body,
        extract::{Path, Query, State},
        http::{Response, StatusCode},
        response::IntoResponse,
        routing::get,
        Json, Router,
    },
    bellscoin::{
        hashes::{sha256, Hash},
        opcodes, script, BlockHash, Network, OutPoint, Transaction, TxOut, Txid,
    },
    db::{RocksDB, RocksTable, UsingConsensus, UsingSerde},
    dutils::{
        async_thread::Spawn,
        error::{ApiError, ContextWrapper},
        wait_token::WaitToken,
    },
    futures::future::join_all,
    inscriptions::Location,
    itertools::Itertools,
    lazy_static::lazy_static,
    num_traits::Zero,
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    serde_with::{serde_as, DisplayFromStr},
    server::{BlockInfo, Server, ServerEvent},
    std::{
        borrow::{Borrow, Cow},
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        fmt::{Display, Formatter},
        future::IntoFuture,
        iter::Peekable,
        marker::PhantomData,
        ops::{Bound, RangeBounds},
        path::PathBuf,
        str::FromStr,
        sync::{atomic::AtomicU64, Arc},
        time::{Duration, Instant},
    },
    tables::DB,
    tokens::*,
    tracing::info,
    tracing_indicatif::span_ext::IndicatifSpanExt,
    utils::{
        address_encoder::{BellscoinDecoder, DogecoinDecoder},
        AsyncClient,
    },
};

mod db;
mod inscriptions;
mod reorg;
mod rest;
mod tables;
mod tokens;
#[macro_use]
mod utils;
mod server;

pub type Fixed128 = nintypes::utils::fixed::Fixed128<18>;

const OP_RETURN_ADDRESS: &str = "BURNED";
const NON_STANDARD_ADDRESS: &str = "non-standard";

lazy_static! {
    static ref OP_RETURN_HASH: FullHash = OP_RETURN_ADDRESS.compute_script_hash();
}

trait IsOpReturnHash {
    fn is_op_return_hash(&self) -> bool;
}

impl IsOpReturnHash for FullHash {
    fn is_op_return_hash(&self) -> bool {
        self.eq(&*OP_RETURN_HASH)
    }
}
lazy_static! {
    static ref BLK_DIR: String = load_env!("BLK_DIR");
    static ref URL: String = load_env!("RPC_URL");
    static ref USER: String = load_env!("RPC_USER");
    static ref PASS: String = load_env!("RPC_PASS");
    static ref BLOCKCHAIN: String = load_env!("BLOCKCHAIN").to_lowercase();
    static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Bellscoin);
    // multiple input inscription scan activation
    static ref JUBILEE_HEIGHT: usize = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 133_000,
        (_, "doge") => usize::MAX,
        _ => 0,
    };
    // first token block height
    static ref START_HEIGHT: u32 = match (*NETWORK, (*BLOCKCHAIN).as_ref()) {
        (Network::Bellscoin, "bells") => 26_371,
        (Network::Bellscoin, "doge") => 4_609_723,
        (Network::Testnet, "doge") => 4_260_514,
        _ => 0,
    };
    static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}

fn main() {
    dotenv::dotenv().ok();
    utils::init_logger();

    dbg!(
        &*BLK_DIR,
        &*URL,
        &*USER,
        &*PASS,
        &*BLOCKCHAIN,
        *NETWORK,
        *JUBILEE_HEIGHT,
        *START_HEIGHT,
        &*SERVER_URL,
        *DEFAULT_HASH
    );

    let indexer_runtime = spawn_runtime("indexer".to_string(), Some(21.try_into().unwrap()));
    indexer_runtime.block_on(async {
        let (raw_event_tx, event_tx, server) = Server::new("rocksdb").await.unwrap();

        let server = Arc::new(server);

        {
            let token = server.token.clone();
            async move {
                tokio::signal::ctrl_c().await.track().ok();
                warn!("Ctrl-C received, shutting down...");
                token.cancel();
            }
            .spawn()
        };

        let server1 = server.clone();

        let rest_server = server.clone();
        std::thread::spawn(move || {
            let rest_runtime = spawn_runtime("rest".to_string(), Some(20.try_into().unwrap()));
            rest_runtime.block_on(run_rest(rest_server.token.clone(), rest_server))
        });

        let threads_handle = server1
            .run_threads(server.token.clone(), raw_event_tx, event_tx)
            .spawn();

        let main_result = inscriptions::main_loop(server.token.clone(), server.clone())
            .spawn()
            .await
            .unwrap();
        server.token.cancel();

        let threads_result = threads_handle.await.unwrap();

        main_result.track().ok();
        threads_result.track().ok();
    })
}

async fn run_rest(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&*SERVER_URL).await.unwrap();

    let rest = axum::serve(listener, rest::get_router(server))
        .with_graceful_shutdown(token.cancelled())
        .into_future();

    let deadline = async move {
        token.cancelled().await;
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

fn spawn_runtime(
    name: String,
    priority: Option<thread_priority::ThreadPriority>,
) -> tokio::runtime::Runtime {
    if let Some(priority) = priority {
        if let Err(e) = thread_priority::set_current_thread_priority(priority) {
            warn!("can't set priority {priority:?}, error {e:?}");
        };
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name(&name)
        .enable_all()
        .build()
        .unwrap();

    runtime
}
