use nint_blk::ScriptType;

use super::*;

pub async fn subscribe(
    State(server): State<Arc<Server>>,
    Json(payload): Json<types::SubscribeArgs>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(200_000);

    let addresses = payload.addresses.unwrap_or_default();

    let tokens = payload.tokens.unwrap_or_default().into_iter().map(LowerCaseTokenTick::from).collect::<HashSet<_>>();

    {
        let mut rx = server.event_sender.subscribe();

        tokio::spawn(async move {
            while !server.token.is_cancelled() {
                match rx.try_recv() {
                    Ok(event) => {
                        match event {
                            ServerEvent::NewHistory(address_token, action) => {
                                if !addresses.is_empty() && !addresses.contains(&address_token.address) {
                                    continue;
                                }

                                if !tokens.is_empty() && !tokens.contains(&address_token.token.into()) {
                                    continue;
                                }

                                let data = Event::default().data(
                                    serde_json::to_string(&types::History {
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
                                    serde_json::to_string(&types::Reorg {
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
                                    serde_json::to_string(&types::NewBlock {
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

pub async fn address_token_history(
    State(server): State<Arc<Server>>,
    Path(script_str): Path<String>,
    Query(query): Query<types::AddressTokenHistoryArgs>,
) -> ApiResult<impl IntoResponse> {
    query.validate().bad_request_from_error()?;

    let scripthash: FullHash = server.indexer.to_scripthash(&script_str, ScriptType::Address).bad_request_from_error()?.into();

    let token: LowerCaseTokenTick = query.tick.into();

    let deploy_proto = server.db.token_to_meta.get(&token).not_found("Token not found")?;

    let token = deploy_proto.proto.tick;

    let from = AddressTokenIdDB {
        address: scripthash,
        id: 0,
        token,
    };

    let to = AddressTokenIdDB {
        address: scripthash,
        id: query.offset.unwrap_or(u64::MAX),
        token,
    };

    let res = server
        .db
        .address_token_to_history
        .range(&from..&to, true)
        .take(query.limit)
        .map(|(k, v)| types::AddressHistory::new(v.height, v.action, k, &server))
        .collect::<anyhow::Result<Vec<_>>>()
        .internal("Failed to load addresses")?;

    Ok(Json(res))
}

pub async fn events_by_height(State(server): State<Arc<Server>>, Path(height): Path<u32>) -> ApiResult<impl IntoResponse> {
    let keys = server.db.block_events.get(height).unwrap_or_default();

    let res = server
        .db
        .address_token_to_history
        .multi_get_kv(keys.iter(), true)
        .into_iter()
        .map(|(k, v)| types::History::new(v.height, v.action, *k, &server))
        .collect::<anyhow::Result<Vec<_>>>()
        .internal("Failed to load addresses")?;

    Ok(Json(res))
}

pub async fn proof_of_history(State(server): State<Arc<Server>>, Query(query): Query<types::ProofHistoryArgs>) -> ApiResult<impl IntoResponse> {
    query.validate().bad_request_from_error()?;

    let res = server
        .db
        .proof_of_history
        .range(..&query.offset.unwrap_or(u32::MAX), true)
        .map(|(height, hash)| types::ProofOfHistory { hash: hash.to_string(), height })
        .take(query.limit)
        .collect_vec();

    Ok(Json(res))
}

pub async fn txid_events(State(server): State<Arc<Server>>, Path(txid): Path<Txid>) -> ApiResult<impl IntoResponse> {
    let keys = server
        .db
        .outpoint_to_event
        .range(&OutPoint { txid, vout: 0 }..&OutPoint { txid, vout: u32::MAX }, false)
        .map(|(_, v)| v)
        .collect_vec();

    let mut events = server
        .db
        .address_token_to_history
        .multi_get_kv(keys.iter(), false)
        .into_iter()
        .map(|(k, v)| types::History::new(v.height, v.action, *k, &server))
        .collect::<anyhow::Result<Vec<_>>>()
        .internal("Failed to load addresses")?;

    events.sort_unstable_by_key(|x| x.address_token.id);

    Ok(Json(events))
}
