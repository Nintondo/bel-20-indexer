use super::*;

pub async fn address_tokens_tick(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path(script_str): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(
            &script_str,
            script_type.parse().bad_request("Invalid script type")?,
        )
        .bad_request("Invalid address")?
        .into();

    let (from, to) = AddressToken::search(scripthash).into_inner();
    let data = state
        .db
        .token_to_meta
        .multi_get(
            state
                .db
                .address_token_to_balance
                .range(&from..&to, false)
                .map(|(k, _)| k.token.into())
                .collect_vec()
                .iter(),
        )
        .into_iter()
        .flatten()
        .map(|x| x.proto.tick.to_string())
        .collect_vec();
    Ok(Json(data))
}

pub async fn address_token_balance(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path((script_str, tick)): Path<(String, String)>,
    Query(params): Query<types::AddressTokenBalanceArgs>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(
            &script_str,
            script_type.parse().bad_request("Invalid script type")?,
        )
        .bad_request("Invalid address")?
        .into();

    let token: LowerCaseTokenTick = tick.into();

    let deploy_proto = state
        .db
        .token_to_meta
        .get(&token)
        .not_found("Token not found")?;

    let tick = deploy_proto.proto.tick;

    let balance = state
        .db
        .address_token_to_balance
        .get(AddressToken {
            address: scripthash,
            token: tick,
        })
        .unwrap_or_default();

    let (from, to) =
        AddressLocation::search(scripthash, params.offset.map(|x| x.into())).into_inner();

    let transfers = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .filter(|(_, v)| v.tick == tick)
        .map(|(k, v)| TokenTransfer {
            amount: v.amt,
            outpoint: k.location.outpoint,
        })
        .take(params.limit)
        .collect_vec();

    let data = types::TokenBalance {
        transfers,
        tick,
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
) -> ApiResult<Response<Body>> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(
            &script_str,
            script_type.parse().bad_request("Invalid script type")?,
        )
        .bad_request("Invalid address")?
        .into();

    let token = params.offset.map(|x| x.0).unwrap_or_default();

    let data = state
        .db
        .address_token_to_balance
        .range(
            &AddressToken {
                address: scripthash,
                token: token.into(),
            }..=&AddressToken {
                address: scripthash,
                token: [u8::MAX; 4].into(),
            },
            false,
        )
        .take(params.limit)
        .map(|(k, v)| types::TokenBalance {
            tick: k.token,
            balance: v.balance,
            transferable_balance: v.transferable_balance,
            transfers_count: v.transfers_count,
            transfers: vec![],
        })
        .collect_vec();

    let data = serde_json::to_vec(&data).internal(INTERNAL)?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Powered-By", "NINTONDO")
        .body(data.into())
        .internal(INTERNAL)
}

pub async fn search_address_tokens(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path((script_str, tick)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let tick = tick.to_lowercase();

    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash: FullHash = state
        .indexer
        .to_scripthash(
            &script_str,
            script_type.parse().bad_request("Invalid script type")?,
        )
        .bad_request("Invalid address")?
        .into();

    let account_tokens = state
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
        .map(|(k, _)| k.token.to_string().to_lowercase())
        .filter(|original_token| original_token.starts_with(&tick))
        .collect_vec();

    let data = serde_json::to_vec(&account_tokens).internal(INTERNAL)?;
    let body = Body::from(data);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Powered-By", "NINTONDO")
        .body(body)
        .internal(INTERNAL)
}
