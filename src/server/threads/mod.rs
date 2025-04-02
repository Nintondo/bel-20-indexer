use super::*;

use dutils::async_thread::{Handler, Thread, ThreadController};

pub mod blocks_loader;
mod event_sender;

impl Server {
    pub async fn run_threads(
        self: Arc<Self>,
        token: WaitToken,
        raw_event_tx: kanal::Receiver<RawServerEvent>,
        event_tx: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> anyhow::Result<()> {
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

        join_all(vec![event_sender])
            .await
            .into_iter()
            .try_collect()
            .anyhow()
    }
}
