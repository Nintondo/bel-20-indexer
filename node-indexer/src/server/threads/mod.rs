use address_hash_saver::AddressesToLoad;
use core_utils::types::server::{RawServerEvent, ServerEvent};
use dutils::async_thread::{Handler, Thread, ThreadController};
use dutils::error::ContextWrapper;
use dutils::wait_token::WaitToken;
use futures::future::join_all;
use itertools::Itertools;
use std::sync::Arc;
use std::time::Duration;
use super::Server;

pub mod address_hash_saver;
pub mod event_sender;

impl Server {
    pub async fn run_threads(
        self: Arc<Self>,
        token: WaitToken,
        addr_rx: kanal::Receiver<AddressesToLoad>,
        raw_event_tx: kanal::Receiver<RawServerEvent>,
        event_tx: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> anyhow::Result<()> {
        let addr_loader = ThreadController::new(address_hash_saver::AddressHasher {
            addr_rx,
            server: self.clone(),
            token: token.clone(),
        })
        .with_name("AddressHasher")
        .with_restart(Duration::from_secs(1))
        .with_cancellation(token.clone())
        .run();

        let event_sender = ThreadController::new(event_sender::EventSender {
            event_tx,
            raw_event_tx,
            server: self.clone(),
            token: token.clone(),
        })
        .with_name("EventSender")
        .with_restart(Duration::from_secs(1))
        .with_cancellation(token)
        .run();

        join_all(vec![addr_loader, event_sender])
            .await
            .into_iter()
            .try_collect()
            .anyhow()
    }
}
