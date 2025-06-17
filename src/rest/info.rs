use super::*;

pub async fn all_addresses(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let addresses = server
            .db
            .address_token_to_balance
            .iter()
            .map(|x| x.0.address)
            .collect::<HashSet<_>>();

        let addresses = server.load_addresses(addresses.iter().copied()).unwrap();

        for address in addresses.iter() {
            if tx.send(address).await.is_err() {
                break;
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

pub async fn all_tokens(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let iter = server.db.token_to_meta.iter().map(|(token, proto)| {
            let tick = String::from_utf8_lossy(token.as_ref()).to_lowercase();
            serde_json::json! ({
                "tick": tick,
                "max": proto.proto.max.to_string(),
                "lim": proto.proto.lim.to_string(),
                "dec": proto.proto.dec,
                "supply": proto.proto.supply.to_string()
            })
        });

        for data in iter {
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

pub async fn status(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let last_height = server
        .db
        .last_block
        .get(())
        .internal("Failed to get last height")?;

    let last_poh = server
        .db
        .proof_of_history
        .get(last_height)
        .internal("Failed to get last proof of history")?;

    let last_block_hash = server
        .db
        .block_info
        .get(last_height)
        .internal("Failed to get last block hash")?
        .hash;

    let data = types::Status {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
    };

    Ok(Json(data))
}
