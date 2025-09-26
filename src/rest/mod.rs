use crate::rest::docs::docs_routes;

use super::*;

use aide::{
    axum::{
        routing::{get, get_with, post_with},
        ApiRouter, IntoApiResponse,
    },
    openapi::{OpenApi, Tag},
    scalar::Scalar,
    transform::{TransformOpenApi, TransformOperation},
};
use axum::{
    extract::{Path, Query, State},
    http::Response,
    http::Uri,
    response::{sse::Event, IntoResponse, Sse},
    Extension, Json,
};
use nintypes::common::inscriptions::Outpoint;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::compression::CompressionLayer;
use validator::Validate;

mod address;
mod docs;
mod history;
mod holders;
mod info;
mod tokens;
pub mod types;
mod utils;
mod wrappers;

pub use wrappers::{OutPoint, Txid};

type ApiResult<T> = core::result::Result<T, Response<String>>;
const INTERNAL: &str = "Internal server error";

pub async fn run_rest(server: Arc<Server>) -> anyhow::Result<()> {
    let token = server.token.clone();

    aide::generate::on_error(|error| {
        println!("{error}");
    });

    aide::generate::extract_schemas(true);

    let mut api = OpenApi::default();

    let listener = tokio::net::TcpListener::bind(&*SERVER_URL).await.unwrap();

    let rest = axum::serve(
        listener,
        ApiRouter::new()
            // Address
            .api_route("/address/{address}", get_with(address::address_tokens, address::address_tokens_docs))
            .api_route("/address/{address}/tokens", get_with(address::address_tokens, address::address_tokens_docs))
            .api_route("/address/{address}/history", get_with(history::address_token_history, history::address_token_history_docs))
            .api_route("/address/{address}/tokens-tick", get_with(address::address_tokens_tick, address::address_tokens_tick_docs))
            .api_route(
                "/address/{address}/{tick}/balance",
                get_with(address::address_token_balance, address::address_token_balance_docs),
            )
            // Token
            .api_route("/tokens", get_with(tokens::tokens, tokens::tokens_docs))
            .api_route("/token", get_with(tokens::token, tokens::token_docs))
            .api_route("/token-supplies", post_with(tokens::token_supplies, tokens::token_supplies_docs))
            .api_route(
                "/token/proof/{address}/{outpoint}",
                get_with(tokens::token_transfer_proof, tokens::token_transfer_proof_docs),
            )
            .api_route("/holders", get_with(holders::holders, holders::holders_docs))
            .api_route("/holders-stats", get_with(holders::holders_stats, holders::holders_stats_docs))
            // Events
            .api_route("/events/{height}", get_with(history::events_by_height, history::events_by_height_docs))
            .api_route("/txid/{txid}", get_with(history::txid_events, history::txid_events_docs))
            .api_route("/token-events/{tick}", get_with(tokens::token_events, tokens::token_events_docs))
            // Status
            .api_route("/status", get_with(info::status, info::status_docs))
            .api_route("/proof-of-history", get_with(history::proof_of_history, history::proof_of_history_docs))
            // Debug
            .nest_api_service("/docs", docs_routes(server.clone()))
            .finish_api_with(&mut api, api_docs)
            // Not documented
            .route("/all-addresses", axum::routing::get(info::all_addresses))
            .route("/all-tickers", axum::routing::get(tokens::all_tickers))
            .route("/events", axum::routing::post(history::subscribe))
            .layer(Extension(Arc::new(api)))
            .layer(CompressionLayer::new())
            .with_state(server),
    )
    .with_graceful_shutdown(token.clone().cancelled())
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

fn api_docs(api: TransformOpenApi) -> TransformOpenApi {
    api.title("BRC-20 Indexer API")
        .tag(Tag {
            name: "address".into(),
            description: Some("Address Management".into()),
            ..Default::default()
        })
        .tag(Tag {
            name: "token".into(),
            description: Some("Token Management".into()),
            ..Default::default()
        })
        .tag(Tag {
            name: "events".into(),
            description: Some("Events Management".into()),
            ..Default::default()
        })
        .tag(Tag {
            name: "status".into(),
            description: Some("Status Management".into()),
            ..Default::default()
        })
}
