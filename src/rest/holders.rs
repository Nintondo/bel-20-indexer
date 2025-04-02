use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use dutils::error::ApiError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::tokens::LowerCaseTick;

use super::{
    utils::{first_page, page_size_default, validate_tick}, ApiResult, Fixed128, Server, BAD_PARAMS,
    INTERNAL,
};

pub async fn holders(
    State(server): State<Arc<Server>>,
    Query(query): Query<Args>,
) -> ApiResult<impl IntoResponse> {
    query.validate().bad_request(BAD_PARAMS)?;

    let tick: LowerCaseTick = query.tick.into();
    let proto = server
        .db
        .token_to_meta
        .get(&tick)
        .map(|x| x.proto)
        .not_found("Tick not found")?;

    let result = if let Some(data) = server.holders.get_holders(&tick) {
        let count = data.len();
        let pages = count.div_ceil(query.page_size);
        let mut holders = Vec::with_capacity(query.page_size);
        let max_percent = data
            .last()
            .map(|x| (x.0 * Fixed128::from(100)).into_decimal() / proto.supply.into_decimal())
            .unwrap_or_default();

        let keys = data
            .iter()
            .rev()
            .enumerate()
            .skip((query.page - 1) * query.page_size)
            .take(query.page_size)
            .map(|(rank, x)| (rank + 1, x.0, x.1));

        for (rank, balance, hash) in keys {
            let address = server.db.fullhash_to_address.get(hash).internal(INTERNAL)?;
            let percent =
                balance.into_decimal() * Decimal::new(100, 0) / proto.supply.into_decimal();

            holders.push(Holder {
                rank,
                address,
                balance: balance.to_string(),
                percent: percent.to_string(),
            })
        }

        Holders {
            pages,
            count,
            max_percent,
            holders,
        }
    } else {
        Holders::default()
    };

    Ok(Json(result))
}

#[derive(Serialize, Deserialize, Default, Validate)]
pub struct Args {
    #[serde(default = "page_size_default")]
    #[validate(range(min = page_size_default(), max = 20))]
    pub page_size: usize,
    #[validate(range(min = 1))]
    #[serde(default = "first_page")]
    pub page: usize,
    #[validate(custom(function = "validate_tick"))]
    pub tick: String,
}

#[derive(Serialize, Deserialize)]
pub struct Holder {
    pub rank: usize,
    pub address: String,
    pub balance: String,
    pub percent: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Holders {
    pub pages: usize,
    pub count: usize,
    pub max_percent: Decimal,
    pub holders: Vec<Holder>,
}
