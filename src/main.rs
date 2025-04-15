extern crate serde;
#[macro_use]
extern crate tracing;

use core_utils::db::{RocksDB, RocksTable, UsingConsensus, UsingSerde};
use core_utils::utils;
use {
    axum::{
        body::Body,
        http::{Response, StatusCode},
        routing::get,
        Router,
    },
    core_utils::tables::DB,
    core_utils::tokens::*,
    dutils::{async_thread::Spawn, error::ContextWrapper, wait_token::WaitToken},
    futures::future::join_all,
    inscriptions::Location,
    itertools::Itertools,
    lazy_static::lazy_static,
    nintondo_dogecoin::{
        hashes::{sha256, Hash}, script, BlockHash, Network, OutPoint,
        TxOut,
        Txid,
    },
    num_traits::Zero,
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    serde_with::{serde_as, DisplayFromStr},
    server::{Server, ServerEvent},
    std::{
        borrow::{Borrow, Cow},
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        fmt::{Display, Formatter},
        future::IntoFuture,
        marker::PhantomData,
        ops::{Bound, RangeBounds},
        str::FromStr,
        sync::{atomic::AtomicU64, Arc},
        time::{Duration, Instant},
    },
    tokio::select,
    tracing::info,
    tracing_indicatif::span_ext::IndicatifSpanExt,
};

mod inscriptions;
mod rest;
mod server;
#[macro_use]
mod core_utils;

pub type Fixed128 = nintypes::utils::fixed::Fixed128<18>;

const MAINNET_START_HEIGHT: u32 = 26_371;

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
    static ref URL: String = load_env!("RPC_URL");
    static ref USER: String = load_env!("RPC_USER");
    static ref PASS: String = load_env!("RPC_PASS");
    static ref NETWORK: Network = load_opt_env!("NETWORK")
        .map(|x| Network::from_str(&x).unwrap())
        .unwrap_or(Network::Dogecoin);
    static ref MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT: usize = if let Network::Dogecoin = *NETWORK
    {
        133_000
    } else {
        0
    };
    static ref START_HEIGHT: u32 = match *NETWORK {
        Network::Dogecoin => MAINNET_START_HEIGHT,
        _ => 0,
    };
    static ref SERVER_URL: String =
        load_opt_env!("SERVER_BIND_URL").unwrap_or("0.0.0.0:8000".to_string());
    static ref DEFAULT_HASH: sha256::Hash = sha256::Hash::hash("null".as_bytes());
}

fn main() {
    dotenv::dotenv().ok();
    utils::init_logger();

    let (raw_event_tx, event_tx, server) = Server::new("rocksdb").unwrap();
    let server = Arc::new(server);

    let rest_server = server.clone();
    std::thread::spawn(move || {
        let future = rest_main(rest_server.token.clone(), rest_server.clone());
        let low_priority_runtime = spawn_runtime("rest".to_string(), None);
        low_priority_runtime.block_on(future)
    });

    let high_priority_runtime = spawn_runtime("indexer".to_string(), Some(21.try_into().unwrap()));

    high_priority_runtime.block_on(indexer_main(server.clone(), raw_event_tx, event_tx));
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

async fn indexer_main(
    server: Arc<Server>,
    rx: kanal::Receiver<Vec<(AddressTokenId, HistoryValue)>>,
    tx: tokio::sync::broadcast::Sender<ServerEvent>,
) {
    let signal_handler = {
        let token = server.token.clone();
        async move {
            select! {
                _ =  token.cancelled() => {}
                _ =  tokio::signal::ctrl_c() => {
                    warn!("Ctrl-C received, shutting down...");
                    token.cancel();
                }
            }

            anyhow::Result::Ok(())
        }
        .spawn()
    };

    let thread_server = server.clone();
    let result = join_all([
        signal_handler,
        thread_server
            .run_threads(server.token.clone(), rx, tx)
            .spawn(),
        async move {
            let main_task = inscriptions::main_loop(server.token.clone(), server.clone())
                .spawn()
                .await?;

            if main_task.is_err() {
                server.token.cancel();
            }

            main_task
        }
        .spawn(),
    ])
    .await;

    let _: Vec<_> = result
        .into_iter()
        .collect::<Result<anyhow::Result<Vec<()>>, _>>()
        .track()
        .unwrap()
        .track()
        .unwrap();
}

async fn rest_main(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    info!("Start REST");

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
