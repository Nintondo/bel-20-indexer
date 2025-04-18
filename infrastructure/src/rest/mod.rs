use axum::routing::get;
use axum::routing::post;
use core_utils::types::rest::load_addresses::AddressesLoader;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::http::Response;
use axum::{
    extract::{Path, Query, State}, http::Uri,
    response::{sse::Event, IntoResponse, Sse},
    Json,
    Router,
};
use validator::Validate;

use dutils::error::ApiError;

use itertools::Itertools;

use nintypes::common::inscriptions::Outpoint;

use rust_decimal::Decimal;

use core_utils::types::rest::rest_api;
use core_utils::types::rest::rest_utils::to_scripthash;
use crate::server::Server;
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

mod address;
mod history;
mod holders;
mod tokens;

type ApiResult<T> = core::result::Result<T, Response<String>>;
const INTERNAL: &str = "Can't handle request";
const BAD_REQUEST: &str = "Can't handle request";
const BAD_PARAMS: &str = "Can't handle request";
const NOT_FOUND: &str = "Can't handle request";

pub fn get_router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/address/{address}", get(address::address_tokens))
        .route("/address/{address}/tokens", get(address::address_tokens))
        .route(
            "/address/{address}/tokens/{tick}",
            get(address::search_address_tokens),
        )
        .route(
            "/address/{address}/history",
            get(history::address_token_history),
        )
        .route(
            "/address/{address}/tokens-tick",
            get(address::address_tokens_tick),
        )
        .route(
            "/address/{address}/{tick}/balance",
            get(address::address_token_balance),
        )
        .route("/tokens", get(tokens::tokens))
        //.route("/token/all", get(all_tokens))
        .route("/token", get(tokens::token))
        .route(
            "/token/proof/{address}/{outpoint}",
            get(tokens::token_transfer_proof),
        )
        .route("/holders", get(holders::holders))
        .route("/events", post(history::subscribe))
        .route("/status", get(status))
        .route("/proof-of-history", get(history::proof_of_history))
        .route("/events/{height}", get(history::events_by_height))
        .route("/all-addresses", get(all_addresses))
        .route("/txid/{txid}", get(history::txid_events))
        .with_state(server)
}

async fn all_addresses(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let addresses = server
            .db
            .address_token_to_balance
            .iter()
            .map(|x| x.0.address)
            .collect::<HashSet<_>>();

        let addresses = server
            .load_addresses(
                addresses.iter().copied(),
                *server.last_indexed_address_height.read().await,
            )
            .await
            .unwrap();

        for (_, address) in addresses {
            if tx.send(address).await.is_err() {
                break;
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

async fn all_tokens(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let iter = server.db.token_to_meta.iter().map(|(token, proto)| {
            let tick = String::from_utf8_lossy(token.as_ref()).to_lowercase();
            serde_json::json! ({
                "genesis": proto.genesis.to_string(),
                "tick": tick,
                "max": proto.proto.max.to_string(),
                "lim": proto.proto.lim.to_string(),
                "dec": proto.proto.dec,
                "transfer_count": proto.proto.transfer_count,
                "mint_count": proto.proto.mint_count
            })
        });

        for data in iter {
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

async fn status(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let last_height = server
        .db
        .last_block
        .get(())
        .internal("Failed to get last height")?;

    let last_poh = server
        .db
        .proof_of_history
        .get(last_height)
        .internal("Failed to get last proof of history")?;

    let last_block_hash = server
        .db
        .block_hashes
        .get(last_height)
        .internal("Failed to get last block hash")?;

    let data = rest_api::Status {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
    };

    Ok(Json(data))
}
