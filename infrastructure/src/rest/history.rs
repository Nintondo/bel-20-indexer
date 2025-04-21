use std::time::Duration;

use super::*;
use application::NETWORK;
use bellscoin::{OutPoint, Txid};
use core_utils::interfaces::server::{DBPort, EventSenderPort, TokenPort};
use core_utils::types::{
    rest::rest_api::{self},
    server::ServerEvent,
    structs::{AddressTokenId, LowerCaseTokenTick},
};
use electrs_indexer::server::Server;
use futures::future::join_all;
use tracing::error;

pub async fn events_by_height<T: DBPort + AddressesLoader + Sized>(
    State(server): State<Arc<T>>,
    Path(height): Path<u32>,
) -> ApiResult<impl IntoResponse> {
    let keys = server.get_db().block_events.get(height).unwrap_or_default();

    let mut res = Vec::<rest_api::History>::new();

    let iterator = server
        .get_db()
        .address_token_to_history
        .multi_get(keys.iter())
        .into_iter()
        .zip(keys);

    for (v, k) in iterator {
        let v = v.not_found("No events found")?;
        res.push(
            rest_api::History::new(v.height, v.action, k, server.as_ref())
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

pub async fn proof_of_history<T: DBPort + ?Sized>(
    State(server): State<Arc<T>>,
    Query(query): Query<rest_api::ProofHistoryArgs>,
) -> ApiResult<impl IntoResponse> {
    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }

    let res = server
        .get_db()
        .proof_of_history
        .range(..&query.offset.unwrap_or(u32::MAX), true)
        .map(|(height, hash)| rest_api::ProofOfHistory {
            hash: hash.to_string(),
            height,
        })
        .take(query.limit.unwrap_or(100))
        .collect_vec();

    Ok(Json(res))
}

pub async fn subscribe<T: DBPort + EventSenderPort + TokenPort + Send + Sync + 'static>(
    State(server): State<Arc<T>>,
    Json(payload): Json<rest_api::SubscribeArgs>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(200_000);

    let addresses = payload.addresses.unwrap_or_default();
    let tokens = payload
        .tokens
        .unwrap_or_default()
        .into_iter()
        .map(LowerCaseTokenTick::from)
        .collect::<HashSet<_>>();

    {
        let mut rx = server.get_event_sender().subscribe();

        tokio::spawn(async move {
            while !server.get_token().is_cancelled() {
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
                                    serde_json::to_string(&rest_api::History {
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
                                    serde_json::to_string(&rest_api::Reorg {
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
                                    serde_json::to_string(&rest_api::NewBlock {
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

pub async fn address_token_history<T>(
    State(server): State<Arc<T>>,
    Path(script_str): Path<String>,
    Query(query): Query<rest_api::AddressTokenHistoryArgs>,
) -> ApiResult<impl IntoResponse>
where
    T: DBPort + AddressesLoader + Send + Sync + 'static,
{
    let scripthash =
        to_scripthash("address", &script_str, *NETWORK).bad_request("Invalid address")?;

    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }
    let token: LowerCaseTokenTick = query.tick.into();

    let deploy_proto = server
        .get_db()
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

    let mut res = Vec::<rest_api::History>::new();

    for (k, v) in server
        .get_db()
        .address_token_to_history
        .range(&from..&to, true)
        .take(query.limit.unwrap_or(100))
        .collect_vec()
    {
        res.push(
            rest_api::History::new(v.height, v.action, k, server.as_ref())
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

pub async fn txid_events<T: DBPort + AddressesLoader + Send + Sync + 'static + Sized>(
    State(server): State<Arc<T>>,
    Path(txid): Path<Txid>,
) -> ApiResult<impl IntoResponse> {
    let keys = server
        .get_db()
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
            .get_db()
            .address_token_to_history
            .multi_get(keys.iter())
            .into_iter()
            .zip(keys)
            .filter_map(|(v, k)| v.map(|v| (k, v)))
            .map(|(k, v)| rest_api::History::new(v.height, v.action, k, server.as_ref())),
    )
    .await
    .into_iter()
    .collect::<anyhow::Result<Vec<_>>>()
    .internal("Failed to load addresses")?;

    events.sort_unstable_by_key(|x| x.address_token.id);

    Ok(Json(events))
}
