use core_utils::utils;
use dutils::async_thread::Spawn;
use dutils::error::ContextWrapper;
use node_indexer::server::Server;
use std::sync::Arc;
use std::time::Duration;
use dutils::wait_token::WaitToken;
use futures::future::join_all;
use tracing::{info, warn};
use application::SERVER_URL;
use infrastructure::rest;
use node_indexer::inscriptions;

fn main() {
    dotenv::dotenv().unwrap();
    utils::init_logger();

    let indexer_runtime = spawn_runtime("indexer".to_string(), Some(21.try_into().unwrap()));
    indexer_runtime.block_on(async {
        let (addr_rx, raw_event_tx, event_tx, server) = Server::new("rocksdb").await.unwrap();

        let server = Arc::new(server);

        let signal_handler = {
            let token = server.token.clone();
            async move {
                tokio::signal::ctrl_c().await.track().ok();
                warn!("Ctrl-C received, shutting down...");
                token.cancel();
                anyhow::Result::Ok(())
            }
            .spawn()
        };

        let server1 = server.clone();

        let rest_server = server.clone();
        std::thread::spawn(move || {
            let rest_runtime = spawn_runtime("rest".to_string(), Some(20.try_into().unwrap()));
            rest_runtime.block_on(run_rest(rest_server.token.clone(), rest_server))
        });

        let result = join_all([
            signal_handler,
            server1
                .run_threads(server.token.clone(), addr_rx, raw_event_tx, event_tx)
                .spawn(),
            inscriptions::main_loop(server.token.clone(), server.clone()).spawn(),
        ])
        .await;

        let _: Vec<_> = result
            .into_iter()
            .collect::<Result<anyhow::Result<Vec<()>>, _>>()
            .track()
            .unwrap()
            .track()
            .unwrap();
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
