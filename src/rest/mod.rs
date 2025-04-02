use axum::{
    response::{sse::Event, Sse},
    routing::post,
};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utils::to_scripthash;

use super::*;

mod address;
mod holders;
mod tokens;
mod utils;

type ApiResult<T> = core::result::Result<T, Response<String>>;
const INTERNAL: &str = "Can't handle request";
const BAD_REQUEST: &str = "Can't handle request";
const BAD_PARAMS: &str = "Can't handle request";
const NOT_FOUND: &str = "Can't handle request";

pub fn get_router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/address/{address}", get(address_tokens))
        .route("/address/{address}/tokens", get(address_tokens))
        .route("/address/{address}/history", get(address_token_history))
        .route(
            "/address/{address}/tokens-tick",
            get(address::address_tokens_tick),
        )
        .route(
            "/address/{address}/{tick}/balance",
            get(address::address_token_balance),
        )
        .route("/tokens", get(tokens::tokens))
        .route("/token/all", get(all_tokens))
        .route("/token", get(tokens::token))
        .route(
            "/token/proof/{address}/{outpoint}",
            get(tokens::token_transfer_proof),
        )
        .route("/holders", get(holders::holders))
        .route("/events", post(subscribe))
        .route("/status", get(status))
        .route("/proof-of-history", get(proof_of_history))
        .route("/events/{height}", get(events_by_height))
        .route("/all-addresses", get(all_addresses))
        .route("/txid/{txid}", get(txid_events))
        .with_state(server)
}

async fn txid_events(
    State(server): State<Arc<Server>>,
    Path(txid): Path<Txid>,
) -> ApiResult<impl IntoResponse> {
    let keys = server
        .db
        .outpoint_to_event
        .range(
            &OutPoint { txid, vout: 0 }..&OutPoint {
                txid,
                vout: u32::MAX,
            },
            false,
        )
        .map(|(_, v)| v)
        .collect_vec();

    let mut events = join_all(
        server
            .db
            .address_token_to_history
            .multi_get(keys.iter())
            .into_iter()
            .zip(keys)
            .filter_map(|(v, k)| v.map(|v| (k, v)))
            .map(|(k, v)| HistoryRest::new(v.height, v.action, k, &server)),
    )
    .await
    .into_iter()
    .collect::<anyhow::Result<Vec<_>>>()
    .internal("Failed to load addresses")?;

    events.sort_unstable_by_key(|x| x.address_token.id);

    Ok(Json(events))
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

    let data = StatusRest {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
    };

    Ok(Json(data))
}

async fn events_by_height(
    State(server): State<Arc<Server>>,
    Path(height): Path<u32>,
) -> ApiResult<impl IntoResponse> {
    let keys = server.db.block_events.get(height).unwrap_or_default();

    let mut res = Vec::<HistoryRest>::new();

    let iterator = server
        .db
        .address_token_to_history
        .multi_get(keys.iter())
        .into_iter()
        .zip(keys);

    for (v, k) in iterator {
        let v = v.not_found("No events found")?;
        res.push(
            HistoryRest::new(v.height, v.action, k, &server)
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

async fn proof_of_history(
    State(server): State<Arc<Server>>,
    Query(query): Query<ProofHistoryParams>,
) -> ApiResult<impl IntoResponse> {
    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }

    let res = server
        .db
        .proof_of_history
        .range(..&query.offset.unwrap_or(u32::MAX), true)
        .map(|(height, hash)| ProofOfHistoryRest {
            hash: hash.to_string(),
            height,
        })
        .take(query.limit.unwrap_or(100))
        .collect_vec();

    Ok(Json(res))
}

async fn subscribe(
    State(server): State<Arc<Server>>,
    Json(payload): Json<SubscribeRequest>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(200_000);

    let addresses = payload.addresses.unwrap_or_default();

    let tokens = payload
        .tokens
        .unwrap_or_default()
        .into_iter()
        .map(LowerCaseTick::from)
        .collect::<HashSet<_>>();

    {
        let mut rx = server.event_sender.subscribe();

        tokio::spawn(async move {
            while !server.token.is_cancelled() {
                match rx.try_recv() {
                    Ok(event) => {
                        match event {
                            ServerEvent::NewHistory(address_token, action) => {
                                if !addresses.is_empty()
                                    && !addresses.contains(&address_token.address)
                                {
                                    continue;
                                }

                                if !tokens.is_empty()
                                    && !tokens.contains(&address_token.token.into())
                                {
                                    continue;
                                }

                                let data = Event::default().data(
                                    serde_json::to_string(&HistoryRest {
                                        address_token: address_token.into(),
                                        height: action.height,
                                        action: action.into(),
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                            ServerEvent::Reorg(blocks_count, new_height) => {
                                let data = Event::default().data(
                                    serde_json::to_string(&ReorgRest {
                                        event_type: "reorg".to_string(),
                                        blocks_count,
                                        new_height,
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                            ServerEvent::NewBlock(height, poh, blockhash) => {
                                let data = Event::default().data(
                                    serde_json::to_string(&NewBlockRest {
                                        event_type: "new_block".to_string(),
                                        height,
                                        proof: poh,
                                        blockhash,
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                        };
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                        error!("Lagged {} events. Disconnecting...", count);
                        break;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                        break;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
    }

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream))
}

async fn address_token_history(
    State(server): State<Arc<Server>>,
    Path(script_str): Path<String>,
    Query(query): Query<AddressTokenHistoryParams>,
) -> ApiResult<impl IntoResponse> {
    let scripthash =
        to_scripthash("address", &script_str, *NETWORK).bad_request("Invalid address")?;

    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }
    let token: LowerCaseTick = query.tick.into();

    let deploy_proto = server
        .db
        .token_to_meta
        .get(&token)
        .not_found("Token not found")?;

    let token = deploy_proto.proto.tick;

    let from = AddressTokenId {
        address: scripthash,
        id: 0,
        token,
    };

    let to = AddressTokenId {
        address: scripthash,
        id: query.offset.unwrap_or(u64::MAX),
        token,
    };

    let mut res = Vec::<HistoryRest>::new();

    for (k, v) in server
        .db
        .address_token_to_history
        .range(&from..&to, true)
        .take(query.limit.unwrap_or(100))
        .collect_vec()
    {
        res.push(
            HistoryRest::new(v.height, v.action, k, &server)
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

async fn address_tokens(
    State(server): State<Arc<Server>>,
    Path(script_str): Path<String>,
) -> ApiResult<Response<Body>> {
    let scripthash =
        to_scripthash("address", &script_str, *NETWORK).bad_request("Invalid address")?;

    let mut ticks = HashMap::<LowerCaseTick, TokenTick>::new();

    let mut data = server
        .db
        .address_token_to_balance
        .range(
            &AddressToken {
                address: scripthash,
                token: [0; 4].into(),
            }..=&AddressToken {
                address: scripthash,
                token: [u8::MAX; 4].into(),
            },
            false,
        )
        .map(|(k, v)| {
            let tick = ticks
                .entry(k.token.clone())
                .or_insert_with(|| server.db.token_to_meta.get(&k.token).unwrap().proto.tick);

            TokenBalanceRest {
                tick: *tick,
                balance: v.balance,
                transferable_balance: v.transferable_balance,
                transfers_count: v.transfers_count,
                transfers: vec![],
            }
        })
        .collect_vec();

    let mut transfers = HashMap::<TokenTick, Vec<(Location, TransferProto)>>::new();

    for (key, value) in server
        .db
        .address_location_to_transfer
        .range(
            &AddressLocation {
                address: scripthash,
                location: Location::zero(),
            }..,
            false,
        )
        .take_while(|x| x.0.address == scripthash)
    {
        transfers
            .entry(value.tick)
            .and_modify(|x| x.push((key.location, value.clone().into())))
            .or_insert(vec![(key.location, value.into())]);
    }

    for token in data.iter_mut() {
        let transfers = transfers
            .remove(&token.tick)
            .unwrap_or_default()
            .into_iter()
            .map(|x| {
                let TransferProto::Bel20 { amt, .. } = x.1;
                TokenTransfer {
                    outpoint: x.0.outpoint,
                    amount: amt,
                }
            })
            .collect();

        token.transfers = transfers;
    }

    let data = serde_json::to_vec(&data).internal(INTERNAL)?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Powered-By", "NINTONDO")
        .body(data.into())
        .internal(INTERNAL)
}

#[derive(Deserialize)]
struct AddressTokenHistoryParams {
    offset: Option<u64>,
    limit: Option<usize>,
    tick: String,
}

#[derive(Deserialize)]
struct SubscribeRequest {
    #[serde(default)]
    addresses: Option<HashSet<String>>,
    #[serde(default)]
    tokens: Option<HashSet<String>>,
}

#[derive(Serialize)]
struct StatusRest {
    height: u32,
    proof: String,
    blockhash: String,
}

#[derive(Serialize)]
struct ProofOfHistoryRest {
    height: u32,
    hash: String,
}

#[derive(Deserialize)]
struct ProofHistoryParams {
    offset: Option<u32>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ReorgRest {
    event_type: String,
    blocks_count: u32,
    new_height: u32,
}

#[derive(Serialize)]
struct NewBlockRest {
    event_type: String,
    height: u32,
    proof: sha256::Hash,
    blockhash: BlockHash,
}
