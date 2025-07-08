use super::*;

pub async fn address_tokens_tick(url: Uri, State(state): State<Arc<Server>>, Path(script_str): Path<String>) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(&script_str, script_type.parse().bad_request("Invalid script type")?)
        .bad_request_from_error()?
        .into();

    let (from, to) = AddressToken::search(scripthash).into_inner();
    let data = state
        .db
        .token_to_meta
        .multi_get(state.db.address_token_to_balance.range(&from..&to, false).map(|(k, _)| k.token.into()).collect_vec().iter())
        .into_iter()
        .flatten()
        .map(|x| x.proto.tick.to_string())
        .collect_vec();
    Ok(Json(data))
}

pub async fn address_token_balance(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path((script_str, tick)): Path<(String, OriginalTokenTickRest)>,
    Query(params): Query<types::AddressTokenBalanceArgs>,
) -> ApiResult<impl IntoResponse> {
    params.validate().bad_request_from_error()?;

    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(&script_str, script_type.parse().bad_request("Invalid script type")?)
        .bad_request_from_error()?
        .into();

    let token: LowerCaseTokenTick = tick.into();

    let deploy_proto = state.db.token_to_meta.get(&token).not_found("Token not found")?;

    let tick = deploy_proto.proto.tick;

    let balance = state.db.address_token_to_balance.get(AddressToken { address: scripthash, token: tick }).unwrap_or_default();

    let (from, to) = AddressLocation::search(scripthash, params.offset.map(|x| x.into())).into_inner();

    let transfers = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .filter(|(_, v)| v.tick == tick)
        .map(|(k, v)| TokenTransfer {
            amount: v.amt,
            outpoint: k.location.outpoint,
        })
        .skip(params.offset.is_some() as usize)
        .take(params.limit)
        .collect_vec();

    let data = types::TokenBalance {
        transfers,
        tick: tick.into(),
        balance: balance.balance,
        transferable_balance: balance.transferable_balance,
        transfers_count: balance.transfers_count,
    };

    Ok(Json(data))
}

pub async fn address_tokens(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path(script_str): Path<String>,
    Query(params): Query<types::AddressTokensArgs>,
) -> ApiResult<impl IntoResponse> {
    params.validate().bad_request_from_error()?;

    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(&script_str, script_type.parse().bad_request("Invalid script type")?)
        .bad_request_from_error()?
        .into();

    let token = params
        .offset
        .as_ref()
        .map(LowerCaseTokenTick::from)
        .and_then(|x| state.db.token_to_meta.get(&x).map(|x| x.proto.tick));

    let data = state
        .db
        .address_token_to_balance
        .range(
            &AddressToken {
                address: scripthash,
                token: token.unwrap_or_default(),
            }..=&AddressToken {
                address: scripthash,
                token: [u8::MAX; 4].into(),
            },
            false,
        )
        .filter(|(k, _)| {
            params
                .search
                .as_ref()
                .map(|x| x.to_lowercase())
                .map(|x| k.token.to_string().to_lowercase().starts_with(&x))
                .unwrap_or(true)
        })
        .skip(params.offset.is_some() as usize)
        .take(params.limit)
        .map(|(k, v)| types::TokenBalance {
            tick: k.token.into(),
            balance: v.balance,
            transferable_balance: v.transferable_balance,
            transfers_count: v.transfers_count,
            transfers: vec![],
        })
        .collect_vec();

    Ok(Json(data))
}
