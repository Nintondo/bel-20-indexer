use super::*;

pub async fn tokens(
    State(server): State<Arc<Server>>,
    Query(args): Query<api::TokensArgs>,
) -> ApiResult<impl IntoResponse> {
    args.validate().bad_request(BAD_PARAMS)?;
    let search = args.search.map(|x| x.to_lowercase().as_bytes().to_vec());

    let iter = server
        .db
        .token_to_meta
        .iter()
        .filter(|x| match args.filter_by {
            api::TokenFilterBy::All => true,
            api::TokenFilterBy::Completed => x.1.is_completed(),
            api::TokenFilterBy::InProgress => !x.1.is_completed(),
        })
        .filter(|x| match &search {
            Some(tick) => x.0.starts_with(tick),
            _ => true,
        });

    let stats = server.holders.stats();
    let all = match args.sort_by {
        api::TokenSortBy::DeployTimeAsc => {
            iter.sorted_by_key(|(_, v)| v.proto.created).collect_vec()
        }
        api::TokenSortBy::DeployTimeDesc => iter
            .sorted_by_key(|(_, v)| v.proto.created)
            .rev()
            .collect_vec(),
        api::TokenSortBy::HoldersAsc => iter
            .sorted_by_key(|(_, v)| stats.get(&v.proto.tick))
            .collect_vec(),
        api::TokenSortBy::HoldersDesc => iter
            .sorted_by_key(|(_, v)| stats.get(&v.proto.tick))
            .rev()
            .collect_vec(),
        api::TokenSortBy::TransactionsAsc => iter
            .sorted_by_key(|(_, v)| v.proto.transactions)
            .collect_vec(),
        api::TokenSortBy::TransactionsDesc => iter
            .sorted_by_key(|(_, v)| v.proto.transactions)
            .rev()
            .collect_vec(),
    };

    let count = all.len();
    let pages = count.div_ceil(args.page_size);
    let tokens = all
        .iter()
        .skip((args.page - 1) * args.page_size)
        .take(args.page_size)
        .map(|(_, v)| api::Token {
            height: v.proto.height,
            created: v.proto.created,
            mint_percent: v.proto.mint_percent().to_string(),
            tick: v.proto.tick,
            genesis: v.genesis,
            deployer: server
                .db
                .fullhash_to_address
                .get(v.proto.deployer)
                .unwrap_or(NON_STANDARD_ADDRESS.to_string()),
            transactions: v.proto.transactions,
            holders: server.holders.holders_by_tick(&v.proto.tick).unwrap_or(0) as u32,
            supply: v.proto.supply,
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .collect_vec();

    Ok(Json(api::TokensResult {
        count,
        pages,
        tokens,
    }))
}

pub async fn token(
    State(back): State<Arc<Server>>,
    Query(args): Query<api::TokenArgs>,
) -> ApiResult<impl IntoResponse> {
    args.validate().bad_request(BAD_REQUEST)?;
    let lower_case_token_tick: LowerCaseTokenTick = args.tick.into();
    let token = back
        .db
        .token_to_meta
        .get(lower_case_token_tick.clone())
        .map(|v| api::Token {
            height: v.proto.height,
            created: v.proto.created,
            deployer: back
                .db
                .fullhash_to_address
                .get(v.proto.deployer)
                .unwrap_or(NON_STANDARD_ADDRESS.to_string()),
            transactions: v.proto.transactions,
            holders: back.holders.holders_by_tick(&v.proto.tick).unwrap_or(0) as u32,
            tick: v.proto.tick,
            genesis: v.genesis,
            supply: v.proto.supply,
            mint_percent: v.proto.mint_percent().to_string(),
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .not_found(NOT_FOUND)?;

    Ok(Json(token))
}

pub async fn token_transfer_proof(
    State(state): State<Arc<Server>>,
    Path((address, outpoint)): Path<(String, Outpoint)>,
) -> ApiResult<impl IntoResponse> {
    let scripthash = to_scripthash("address", &address, *NETWORK).bad_request("Invalid address")?;

    let (from, to) = AddressLocation::search(scripthash, Some(outpoint.into())).into_inner();

    let data: Vec<_> = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .map(|(_, TransferProtoDB { tick, amt, height })| {
            anyhow::Ok(api::TokenTransferProof { amt, tick, height })
        })
        .try_collect()
        .track_with("")
        .internal(INTERNAL)?;

    Ok(Json(data))
}
