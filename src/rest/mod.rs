use super::*;

use axum::{
    http::Uri,
    response::{sse::Event, Sse},
    routing::post,
};
use nintypes::common::inscriptions::Outpoint;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tower_http::compression::CompressionLayer;
use validator::Validate;

mod address;
mod history;
mod holders;
mod info;
mod tokens;
pub mod types;
mod utils;

type ApiResult<T> = core::result::Result<T, Response<String>>;
const INTERNAL: &str = "Internal server error";

pub fn get_router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/address/{address}", get(address::address_tokens))
        .route("/address/{address}/tokens", get(address::address_tokens))
        .route("/address/{address}/history", get(history::address_token_history))
        .route("/address/{address}/tokens-tick", get(address::address_tokens_tick))
        .route("/address/{address}/{tick}/balance", get(address::address_token_balance))
        .route("/tokens", get(tokens::tokens))
        .route("/token", get(tokens::token))
        .route("/token/proof/{address}/{outpoint}", get(tokens::token_transfer_proof))
        .route("/inscriptions/{outpoint}", get(info::inscriptions_on_outpoint))
        .route("/holders", get(holders::holders))
        .route("/holders-stats", get(holders::holders_stats))
        .route("/events", post(history::subscribe))
        .route("/status", get(info::status))
        .route("/proof-of-history", get(history::proof_of_history))
        .route("/events/{height}", get(history::events_by_height))
        .route("/all-addresses", get(info::all_addresses))
        .route("/txid/{txid}", get(history::txid_events))
        .with_state(server)
        .layer(CompressionLayer::new())
}
