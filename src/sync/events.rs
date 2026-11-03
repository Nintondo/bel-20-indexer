use crate::{Server, ServerEvent};
use axum::response::IntoResponse;
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive},
        Sse,
    },
};
use nintondo_holdings_protocol::SyncEvent;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::ReceiverStream;

/// Notifications only. Consumers recover data through the durable block feed.
pub async fn subscribe(State(server): State<Arc<Server>>) -> axum::response::Response {
    let mut events = server.event_sender.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(32);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Event::default().event("sync").json_data(SyncEvent::Ready).expect("serialize notification")))
            .await;
        loop {
            let event = tokio::select! {
                _ = tx.closed() => break,
                _ = server.token.cancelled() => break,
                event = events.recv() => match event { Ok(event) => event, Err(_) => break },
            };
            let data = match event {
                ServerEvent::NewBlock(height, _, hash) => SyncEvent::NewBlock {
                    height,
                    blockhash: hash.to_string(),
                },
                ServerEvent::Reorg(count, height) => SyncEvent::Reorg {
                    blocks_count: count,
                    new_height: height,
                },
                ServerEvent::NewHistory(..) => continue,
            };
            if tx.try_send(Ok(Event::default().event("sync").json_data(data).expect("serialize notification"))).is_err() {
                break;
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()).into_response()
}
