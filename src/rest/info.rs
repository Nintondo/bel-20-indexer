use super::*;

pub async fn all_addresses(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let mut last_address: Option<FullHash> = None;
        for fullhash in server.db.address_token_to_balance.iter().map(|x| x.0.address) {
            if last_address.is_some_and(|x| x == fullhash) {
                continue;
            }

            let Some(address_str) = server.db.fullhash_to_address.get(fullhash) else {
                continue;
            };

            if tx.send(address_str).await.is_err() {
                break;
            }

            last_address = Some(fullhash);
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

pub async fn status(State(server): State<Arc<Server>>) -> ApiResult<impl IntoApiResponse> {
    let last_height = server.db.last_block.get(()).internal("Failed to get last height")?;

    let last_poh = server.db.proof_of_history.get(last_height).internal("Failed to get last proof of history")?;

    let last_block_hash = server.db.block_info.get(last_height).internal("Failed to get last block hash")?.hash;

    let data = types::Status {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
        version: PKG_VERSION.to_string(),
        uptime_secs: server.start_time.elapsed().as_secs(),
    };

    Ok(Json(data))
}

pub fn status_docs(op: TransformOperation) -> TransformOperation {
    op.description("Status of the indexer").tag("status")
}
