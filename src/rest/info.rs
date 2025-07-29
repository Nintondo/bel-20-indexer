use super::*;

pub async fn all_addresses(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let addresses = server.db.address_token_to_balance.iter().map(|x| x.0.address).collect::<HashSet<_>>();

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

pub async fn status(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let last_height = server.db.last_block.get(()).internal("Failed to get last height")?;

    let last_poh = server.db.proof_of_history.get(last_height).internal("Failed to get last proof of history")?;

    let last_block_hash = server.db.block_info.get(last_height).internal("Failed to get last block hash")?.hash;

    let data = types::Status {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
        version: PKG_VERSION.to_string(),
    };

    Ok(Json(data))
}

pub async fn inscriptions_on_outpoint(State(server): State<Arc<Server>>, Path(outpoint): Path<Outpoint>) -> ApiResult<impl IntoResponse> {
    Ok(Json(server.db.outpoint_to_inscription_offsets.get(OutPoint::from(outpoint)).unwrap_or_default()))
}
