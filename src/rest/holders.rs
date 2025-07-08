use super::*;

pub async fn holders(State(server): State<Arc<Server>>, Query(query): Query<types::HoldersArgs>) -> ApiResult<impl IntoResponse> {
    query.validate().bad_request_from_error()?;

    let tick: LowerCaseTokenTick = query.tick.into();
    let proto = server.db.token_to_meta.get(&tick).map(|x| x.proto).not_found("Tick not found")?;

    let result = if let Some(data) = server.holders.get_holders(&proto.tick) {
        let count = data.len();
        let pages = count.div_ceil(query.page_size);
        let mut holders = Vec::with_capacity(query.page_size);
        let max_percent = data.last().map(|x| x.0 / proto.supply * Fixed128::from(100)).unwrap_or_default();

        let keys = data
            .iter()
            .rev()
            .enumerate()
            .skip((query.page - 1) * query.page_size)
            .take(query.page_size)
            .map(|(rank, x)| (rank + 1, x.0, x.1));

        for (rank, balance, hash) in keys {
            let address = server.db.fullhash_to_address.get(hash).internal(INTERNAL)?;
            let percent = balance / proto.supply * Fixed128::from(100);

            holders.push(types::Holder {
                rank,
                address,
                balance: balance.to_string(),
                percent: percent.to_string(),
            })
        }

        types::Holders {
            pages,
            count,
            max_percent: max_percent.to_string(),
            holders,
        }
    } else {
        types::Holders::default()
    };

    Ok(Json(result))
}

pub async fn holders_stats(State(server): State<Arc<Server>>, Query(query): Query<types::HoldersStatsArgs>) -> ApiResult<impl IntoResponse> {
    let tick: LowerCaseTokenTick = query.tick.into();
    let proto = server.db.token_to_meta.get(&tick).map(|x| x.proto).not_found("Tick not found")?;

    let result = if let Some(data) = server.holders.get_holders(&proto.tick) {
        let mut result = Vec::with_capacity(5);

        let mut iter = data.iter().rev().map(|x| x.0);

        let mut total_value = Fixed128::ZERO;

        for limit in [100, 100, 200, 500] {
            let mut value = Fixed128::ZERO;
            for _ in 0..limit {
                value += iter.next().unwrap_or_default();
            }

            total_value += value;

            let percent = value / proto.supply * Fixed128::from(100);
            result.push(percent);
        }

        let left = (proto.supply - total_value) / proto.supply * Fixed128::from(100);
        result.push(left);

        result
    } else {
        vec![]
    };

    Ok(Json(result))
}
