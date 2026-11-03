use crate::{Server, ServerEvent};
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive},
        Sse,
    },
};
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::ReceiverStream;

/// Notifications only. Consumers recover data through the durable block feed.
pub async fn subscribe(State(server): State<Arc<Server>>) -> impl axum::response::IntoResponse {
    let mut events = server.event_sender.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(32);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Event::default().event("sync").data("{\"event_type\":\"ready\"}"))).await;
        loop {
            let event = tokio::select! {
                _ = server.token.cancelled() => break,
                event = events.recv() => match event { Ok(event) => event, Err(_) => break },
            };
            let data = match event {
                ServerEvent::NewBlock(height, _, hash) => serde_json::json!({"event_type":"new_block", "height":height, "blockhash":hash}),
                ServerEvent::Reorg(count, height) => serde_json::json!({"event_type":"reorg", "blocks_count":count, "new_height":height}),
                ServerEvent::NewHistory(..) => continue,
            };
            if tx.try_send(Ok(Event::default().event("sync").data(data.to_string()))).is_err() {
                break;
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}
