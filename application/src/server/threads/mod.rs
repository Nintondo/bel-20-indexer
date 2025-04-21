use blocks_loader::BlocksLoader;
use core_utils::types::loaded_blocks::LoadedBlocks;
use core_utils::types::server::{RawServerEvent, ServerEvent};
use core_utils::types::token_history::TokenHistoryData;
use dutils::async_thread::{Handler, Thread, ThreadController};
use dutils::error::ContextWrapper;
use dutils::wait_token::WaitToken;
use futures::future::join_all;
use itertools::Itertools;
use std::sync::Arc;
use std::time::Duration;

use super::Server;

pub mod blocks_loader;
mod event_sender;

impl Server {
    pub async fn run_threads(
        self: Arc<Self>,
        token: WaitToken,
        raw_event_tx: kanal::Receiver<RawServerEvent>,
        event_tx: tokio::sync::broadcast::Sender<ServerEvent>,
        blocks_storage: Arc<tokio::sync::Mutex<LoadedBlocks>>,
        client: Arc<electrs_client::Client<TokenHistoryData>>,
    ) -> anyhow::Result<()> {
        let event_sender = ThreadController::new(event_sender::EventSender {
            event_tx,
            raw_event_tx,
            server: self.clone(),
            token: token.clone(),
        })
        .with_name("EventSender")
        .with_restart(Duration::from_secs(1))
        .with_cancellation(token.clone())
        .run();

        let blocks_loader = dutils::async_thread::ThreadController::new(BlocksLoader {
            storage: blocks_storage.clone(),
            client: client.clone(),
        })
        .with_name("BlocksLoader")
        .with_restart(Duration::from_secs(5))
        .with_invoke_frq(Duration::from_millis(100))
        .with_cancellation(token.clone())
        .run();

        join_all(vec![event_sender, blocks_loader])
            .await
            .into_iter()
            .try_collect()
            .anyhow()
    }
}
