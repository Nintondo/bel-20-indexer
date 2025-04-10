use super::*;

pub async fn holders(
    State(server): State<Arc<Server>>,
    Query(query): Query<api::HoldersArgs>,
) -> ApiResult<impl IntoResponse> {
    query.validate().bad_request(BAD_PARAMS)?;

    let tick: LowerCaseTokenTick = query.tick.into();
    let proto = server
        .db
        .token_to_meta
        .get(&tick)
        .map(|x| x.proto)
        .not_found("Tick not found")?;

    let result = if let Some(data) = server.holders.get_holders(&proto.tick) {
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

            holders.push(api::Holder {
                rank,
                address,
                balance: balance.to_string(),
                percent: percent.to_string(),
            })
        }

        api::Holders {
            pages,
            count,
            max_percent,
            holders,
        }
    } else {
        api::Holders::default()
    };

    Ok(Json(result))
}
