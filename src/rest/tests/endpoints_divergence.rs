#![allow(clippy::uninlined_format_args)]

use super::*;

#[tokio::test(flavor = "current_thread"])
async fn endpoints_equal_poh_and_events_but_token_differs() {
    // Set minimal env so Server::new initializes
    std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
    std::env::set_var("RPC_USER", "user");
    std::env::set_var("RPC_PASS", "pass");
    std::env::set_var("BLOCKCHAIN", "bellscoin");

    // Two nodes
    let t1 = tempfile::tempdir().expect("tempdir");
    let (_raw_rx_a, _tx_a, server_a0) = Server::new(t1.path().to_str().unwrap()).expect("server");
    let server_a = std::sync::Arc::new(server_a0);

    let t2 = tempfile::tempdir().expect("tempdir");
    let (_raw_rx_b, _tx_b, server_b0) = Server::new(t2.path().to_str().unwrap()).expect("server");
    let server_b = std::sync::Arc::new(server_b0);

    // Write identical History + Info to both
    let height = *START_HEIGHT + 20;
    let tick = OriginalTokenTick::from_str("REST").unwrap();
    let key = AddressTokenIdDB { address: FullHash::ZERO, token: tick, id: 1 };
    let hv = HistoryValue { height, action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 } };
    let history = vec![(key, hv)];
    let prev = *DEFAULT_HASH;
    let addresses = AddressesFullHash::new(std::collections::HashMap::new());
    let poh = Server::generate_history_hash(prev, &history, &addresses).unwrap();

    let info = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
    ProcessedData::History { block_number: height, last_history_id: 1, history: history.clone() }.write(&server_a, None);
    ProcessedData::Info { block_number: height, block_info: info, block_proof: poh }.write(&server_a, None);

    let info_b = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
    ProcessedData::History { block_number: height, last_history_id: 1, history: history.clone() }.write(&server_b, None);
    ProcessedData::Info { block_number: height, block_info: info_b, block_proof: poh }.write(&server_b, None);

    // Only node A writes snapshots
    let meta = TokenMetaDB {
        genesis: InscriptionId { txid: Txid::all_zeros(), index: 0 },
        proto: DeployProtoDB {
            tick,
            max: Fixed128::from(0),
            lim: Fixed128::from(0),
            dec: 0,
            supply: Fixed128::from(0),
            transfer_count: 0,
            mint_count: 0,
            height,
            created: 0,
            deployer: FullHash::ZERO,
            transactions: 0,
        },
    };
    ProcessedData::Tokens {
        metas: vec![(LowerCaseTokenTick::from(tick), meta)],
        balances: vec![],
        transfers_to_write: vec![],
        transfers_to_remove: vec![],
    }.write(&server_a, None);

    // 1) proof_of_history: equal
    let a_resp = history::proof_of_history(State(server_a.clone()), Query(types::ProofHistoryArgs { offset: Some(height), limit: 10 })).await.unwrap().into_response();
    let b_resp = history::proof_of_history(State(server_b.clone()), Query(types::ProofHistoryArgs { offset: Some(height), limit: 10 })).await.unwrap().into_response();
    let a_body = axum::body::to_bytes(a_resp.into_body(), 64 * 1024).await.unwrap();
    let b_body = axum::body::to_bytes(b_resp.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(a_body, b_body, "PoH responses should match");

    // 2) events_by_height: equal
    let a_resp = history::events_by_height(State(server_a.clone()), Path(height)).await.unwrap().into_response();
    let b_resp = history::events_by_height(State(server_b.clone()), Path(height)).await.unwrap().into_response();
    let a_body = axum::body::to_bytes(a_resp.into_body(), 64 * 1024).await.unwrap();
    let b_body = axum::body::to_bytes(b_resp.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(a_body, b_body, "events_by_height responses should match");

    // 3) token endpoint: Node A returns Ok; Node B is Err (404)
    let args = types::TokenArgs { tick: tick.into() };
    let a_token = tokens::token(State(server_a), Query(args.clone())).await;
    let b_token = tokens::token(State(server_b), Query(args)).await;
    assert!(a_token.is_ok(), "token endpoint A should succeed");
    assert!(b_token.is_err(), "token endpoint B should error");
}

