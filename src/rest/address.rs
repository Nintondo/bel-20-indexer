use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::Uri,
    response::IntoResponse,
    Json,
};
use dutils::error::ApiError;
use itertools::Itertools;
use nintypes::common::inscriptions::Outpoint;
use serde::{Deserialize, Serialize};

use crate::LowerCaseTick;

use super::{
    utils::to_scripthash, AddressLocation, AddressToken, ApiResult, Fixed128, FullHash, Server, TokenTick,
    TokenTransfer, INTERNAL, NETWORK,
};

pub async fn address_tokens_tick(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path(script_str): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash =
        to_scripthash(script_type, &script_str, *NETWORK).bad_request("Invalid address")?;
    let (from, to) = AddressToken::search(scripthash).into_inner();
    let data = state
        .db
        .token_to_meta
        .multi_get(
            state
                .db
                .address_token_to_balance
                .range(&from..&to, false)
                .map(|(k, _)| k.token)
                .collect_vec()
                .iter(),
        )
        .into_iter()
        .flatten()
        .map(|x| x.proto.tick)
        .collect_vec();
    Ok(Json(data))
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash)]
pub struct AddressTick {
    pub address: FullHash,
    pub tick: TokenTick,
}

pub async fn address_token_balance(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path((script_str, tick)): Path<(String, String)>,
    Query(params): Query<AddressTokenBalanceArgs>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash =
        to_scripthash(script_type, &script_str, *NETWORK).bad_request("Invalid address")?;

    let token: LowerCaseTick = tick.into();

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
            token: tick.into(),
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
        .collect_vec();

    let data = TokenBalance {
        transfers,
        tick,
        balance: balance.balance,
        transferable_balance: balance.transferable_balance,
        transfers_count: balance.transfers_count,
    };

    Ok(Json(data))
}

#[derive(Deserialize)]
pub struct AddressTokenBalanceArgs {
    pub offset: Option<Outpoint>,
}

#[derive(Serialize, Deserialize)]
pub struct TokenBalance {
    pub tick: TokenTick,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}
