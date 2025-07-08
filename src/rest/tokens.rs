use nint_blk::ScriptType;

use super::*;

pub async fn tokens(State(server): State<Arc<Server>>, Query(args): Query<types::TokensArgs>) -> ApiResult<impl IntoResponse> {
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
            genesis: v.genesis,
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

pub async fn token(State(server): State<Arc<Server>>, Query(args): Query<types::TokenArgs>) -> ApiResult<impl IntoResponse> {
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
            genesis: v.genesis,
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

pub async fn token_transfer_proof(State(state): State<Arc<Server>>, Path((address, outpoint)): Path<(String, Outpoint)>) -> ApiResult<impl IntoResponse> {
    let scripthash = state.indexer.to_scripthash(&address, ScriptType::Address).bad_request_from_error()?;

    let (from, to) = AddressLocation::search_with_offset(scripthash.into(), outpoint.into()).into_inner();

    let data: Vec<_> = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .map(|(_, TransferProtoDB { tick, amt, height })| anyhow::Ok(types::TokenTransferProof { amt, tick: tick.into(), height }))
        .try_collect()
        .track_with("")
        .internal(INTERNAL)?;

    Ok(Json(data))
}
