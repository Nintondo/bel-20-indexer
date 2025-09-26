use axum::http::StatusCode;
use bitcoin_hashes::sha256d;
use nint_blk::ScriptType;

use super::*;

pub async fn tokens(State(server): State<Arc<Server>>, Query(args): Query<types::TokensArgs>) -> ApiResult<impl IntoApiResponse> {
    args.validate().bad_request_from_error()?;

    let iter = server
        .db
        .token_to_meta
        .iter()
        .filter(|x| match args.filter_by {
            types::TokenFilterBy::All => true,
            types::TokenFilterBy::Completed => x.1.is_completed(),
            types::TokenFilterBy::InProgress => !x.1.is_completed(),
        })
        .filter(|x| args.search.as_ref().map(|tick| x.0.starts_with(tick)).unwrap_or(true));

    let stats = server.holders.stats();
    let all = match args.sort_by {
        types::TokenSortBy::DeployTimeAsc => iter.sorted_by_key(|(_, v)| v.proto.created).collect_vec(),
        types::TokenSortBy::DeployTimeDesc => iter.sorted_by_key(|(_, v)| v.proto.created).rev().collect_vec(),
        types::TokenSortBy::HoldersAsc => iter.sorted_by_key(|(_, v)| stats.get(&v.proto.tick)).collect_vec(),
        types::TokenSortBy::HoldersDesc => iter.sorted_by_key(|(_, v)| stats.get(&v.proto.tick)).rev().collect_vec(),
        types::TokenSortBy::TransactionsAsc => iter.sorted_by_key(|(_, v)| v.proto.transactions).collect_vec(),
        types::TokenSortBy::TransactionsDesc => iter.sorted_by_key(|(_, v)| v.proto.transactions).rev().collect_vec(),
    };

    let count = all.len();
    let pages = count.div_ceil(args.page_size);
    let tokens = all
        .iter()
        .skip((args.page - 1) * args.page_size)
        .take(args.page_size)
        .map(|(_, v)| types::Token {
            height: v.proto.height,
            created: v.proto.created,
            mint_percent: v.proto.mint_percent().to_string(),
            tick: v.proto.tick.into(),
            genesis: v.genesis.into(),
            deployer: fullhash_to_address_str(&v.proto.deployer, server.db.fullhash_to_address.get(v.proto.deployer)),
            transactions: v.proto.transactions,
            holders: server.holders.holders_by_tick(&v.proto.tick).unwrap_or(0) as u32,
            supply: v.proto.supply,
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .collect_vec();

    Ok(Json(types::TokensResult { count, pages, tokens }))
}

pub fn tokens_docs(op: TransformOperation) -> TransformOperation {
    op.description("A complete list of tokens with sorts, filters and search").tag("token")
}

pub async fn token(State(server): State<Arc<Server>>, Query(args): Query<types::TokenArgs>) -> ApiResult<impl IntoApiResponse> {
    args.validate().bad_request_from_error()?;

    let lower_case_token_tick: LowerCaseTokenTick = args.tick.into();
    let token = server
        .db
        .token_to_meta
        .get(lower_case_token_tick.clone())
        .map(|v| types::Token {
            height: v.proto.height,
            created: v.proto.created,
            deployer: fullhash_to_address_str(&v.proto.deployer, server.db.fullhash_to_address.get(v.proto.deployer)),
            transactions: v.proto.transactions,
            holders: server.holders.holders_by_tick(&v.proto.tick).unwrap_or(0) as u32,
            tick: v.proto.tick.into(),
            genesis: v.genesis.into(),
            supply: v.proto.supply,
            mint_percent: v.proto.mint_percent().to_string(),
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .not_found(format!("Tick {} not found", args.tick))?;

    Ok(Json(token))
}

pub fn token_docs(op: TransformOperation) -> TransformOperation {
    op.description("Detailed information about a token").tag("token")
}

pub async fn token_supplies(State(server): State<Arc<Server>>, Json(ticks): Json<Vec<OriginalTokenTickRest>>) -> ApiResult<impl IntoApiResponse> {
    let keys = ticks.into_iter().map(LowerCaseTokenTick::from).collect_vec();
    let res = server
        .db
        .token_to_meta
        .multi_get(keys.iter())
        .into_iter()
        .map(|x| x.map(|x| x.proto.supply.to_string()))
        .collect::<Option<Vec<_>>>()
        .not_found("Some of ticks is invalid")?;

    Ok(Json(res))
}

pub fn token_supplies_docs(op: TransformOperation) -> TransformOperation {
    op.description("Batch operation to get token supply for each token from the provided list").tag("token")
}

pub async fn token_transfer_proof(State(state): State<Arc<Server>>, Path((address, outpoint)): Path<(String, Outpoint)>) -> ApiResult<impl IntoApiResponse> {
    let scripthash = state.indexer.to_scripthash(&address, ScriptType::Address).bad_request_from_error()?;

    let (from, to) = AddressLocation::search_with_offset(scripthash.into(), outpoint.into()).into_inner();

    let start = Instant::now();

    while start.elapsed() < Duration::from_secs(5) {
        let best_block_hash = state.client.get_best_block_hash().internal("Failed to connect to node")?;
        let last_block = state.db.last_block.get(()).internal("Failed to get last block")?;
        let last_block_hash: sha256d::Hash = state.db.block_info.get(last_block).internal("Failed to get last block info")?.hash.into();

        if best_block_hash == last_block_hash {
            let data: Vec<_> = state
                .db
                .address_location_to_transfer
                .range(&from..&to, false)
                .map(|(_, TransferProtoDB { tick, amt, height })| anyhow::Ok(types::TokenTransferProof { amt, tick: tick.into(), height }))
                .try_collect()
                .track_with("")
                .internal(INTERNAL)?;

            return Ok(Json(data));
        }
    }

    let res = axum::response::Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body("Service isn't synced".into())
        .internal("Failed to build body for the response")?;

    Err(res)
}

pub fn token_transfer_proof_docs(op: TransformOperation) -> TransformOperation {
    op.description("Verifies a transfer by address and outpoint").tag("token")
}

pub async fn token_events(
    State(server): State<Arc<Server>>,
    Path(token): Path<OriginalTokenTickRest>,
    Query(args): Query<types::TokenEventsArgs>,
) -> ApiResult<impl IntoApiResponse> {
    if let Some(outpoint_str) = args.search {
        let txid = Txid::from_str(&outpoint_str[..64.min(outpoint_str.len())]).bad_request_from_error()?;

        let vout: Option<u32> = if outpoint_str.len() > 65 {
            // Skip 64th byte because it's separator
            Some(outpoint_str[65..].parse().bad_request("Failed to parse outpoint from search prompt")?)
        } else {
            None
        };

        let from = bellscoin::OutPoint {
            txid: *txid,
            vout: vout.unwrap_or(0),
        };
        let to = bellscoin::OutPoint {
            txid: *txid,
            vout: vout.unwrap_or(u32::MAX),
        };

        let v = server
            .db
            .outpoint_to_event
            .range(&from..=&to, false)
            .take(args.limit)
            .flat_map(|(_, x)| server.db.address_token_to_history.get(x).map(|v| (x, v)))
            .map(|(k, v)| types::AddressHistory::new(v.height, v.action, k, &server))
            .collect::<Result<Vec<_>, _>>()
            .internal("Couldn't found block for history entry")?;

        Ok(Json(v))
    } else {
        let from = TokenId { id: 0, token: token.into() };

        let offset = args.offset.unwrap_or(u64::MAX);
        let to = TokenId { id: offset, token: token.into() };

        let keys = server.db.token_id_to_event.range(&from..&to, true).take(args.limit).map(|x| x.1).collect_vec();
        let history = server
            .db
            .address_token_to_history
            .multi_get_kv(keys.iter(), false)
            .into_iter()
            .map(|(k, v)| types::AddressHistory::new(v.height, v.action, *k, &server))
            .collect::<Result<Vec<_>, _>>()
            .internal("Couldn't found block for history entry")?;
        Ok(Json(history))
    }
}

pub fn token_events_docs(op: TransformOperation) -> TransformOperation {
    op.description("A complete list of token events sorted by date of creation").tag("token")
}

pub async fn all_tickers(State(server): State<Arc<Server>>, Query(args): Query<types::AllTickersQuery>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    tokio::spawn(async move {
        if let Some(height) = args.block_height {
            if let Some(events) = server.db.block_events.get(height) {
                for x in server.db.address_token_to_history.multi_get_kv(events.iter(), true).into_iter().filter_map(|(k, v)| {
                    if let TokenHistoryDB::Deploy { .. } = v.action {
                        Some(k.token)
                    } else {
                        None
                    }
                }) {
                    if tx.send(x.to_string()).await.is_err() {
                        break;
                    }
                }
            }
        } else {
            for (_, meta) in server.db.token_to_meta.iter() {
                if tx.send(meta.proto.tick.to_string()).await.is_err() {
                    break;
                }
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}
