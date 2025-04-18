use std::{sync::Arc, time::Duration};

use core_utils::{
    types::{
        server::ServerEvent,
        structs::{AddressTokenId, HistoryValue},
    },
    utils,
};
use dutils::{async_thread::Spawn, error::ContextWrapper, wait_token::WaitToken};
use electrs_indexer::{inscriptions::main_loop, SERVER_URL};
use futures::future::join_all;
use tokio::select;
use tracing::{info, warn};
use server::Server;

mod rest;
pub mod server;

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    println!("Version: {}", version);
    
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
            let main_task = main_loop(server.token.clone(), server.clone())
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
