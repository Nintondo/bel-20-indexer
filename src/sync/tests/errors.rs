use super::*;
use axum::{body::Body, http::Request, Router};
use rocksdb_wrapper::Pebble;
use tower::ServiceExt;

#[tokio::test]
async fn authentication_precedes_snapshot_allocation() {
    let (_fixture, server) = setup();
    let app = Router::new()
        .nest("/sync", crate::sync::routes::router(server.clone()).finish_api(&mut Default::default()))
        .with_state(server.clone());
    for authorization in [None, Some("Bearer incorrect-key"), Some("Basic incorrect-key"), Some("Bearer ")] {
        for (method, path) in [
            ("POST", "/sync/snapshot"),
            ("GET", "/sync/snapshot/unknown"),
            ("DELETE", "/sync/snapshot/unknown"),
            ("GET", "/sync/status"),
            ("GET", "/sync/events"),
            ("GET", "/sync/blocks"),
            ("GET", "/sync/block/30000"),
        ] {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(value) = authorization {
                request = request.header("Authorization", value);
            }
            let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), 401, "{method} {path}");
        }
    }
    for _ in 0..crate::sync::config::SNAPSHOT_LIMIT {
        assert_eq!(request(server.clone(), "POST", "/sync/snapshot").await.0, 200);
    }
    assert_eq!(request(server, "POST", "/sync/snapshot").await.0, 429);
}

#[tokio::test]
async fn snapshot_internal_errors_are_500_and_do_not_expire_the_session() {
    let (_fixture, server) = setup();
    seed(&server, 1, *b"AAAA", 10);
    let meta = server.db.token_to_meta.get(LowerCaseTokenTick::from(*b"AAAA")).unwrap();
    server.db.token_to_meta.remove(LowerCaseTokenTick::from(*b"AAAA"));
    let (_, body) = request(server.clone(), "POST", "/sync/snapshot").await;
    let created: SnapshotCreated = serde_json::from_slice(&body).unwrap();
    let path = format!("/sync/snapshot/{}", created.id);
    assert_eq!(request(server.clone(), "GET", &path).await.0, 500);
    assert_eq!(request(server.clone(), "GET", &path).await.0, 500);
    assert_eq!(request(server.clone(), "GET", "/sync/snapshot/unknown").await.0, 410);
    assert_eq!(request(server.clone(), "GET", &format!("{path}?cursor=bad")).await.0, 400);
    assert_eq!(request(server.clone(), "GET", &format!("{path}?limit=1001")).await.0, 400);
    server.db.token_to_meta.set(LowerCaseTokenTick::from(*b"AAAA"), meta);
    // The old snapshot remains an immutable view of the missing metadata.
    assert_eq!(request(server.clone(), "GET", &path).await.0, 500);
    let (_, body) = request(server.clone(), "POST", "/sync/snapshot").await;
    let created: SnapshotCreated = serde_json::from_slice(&body).unwrap();
    assert_eq!(request(server, "GET", &format!("/sync/snapshot/{}", created.id)).await.0, 200);
}

#[tokio::test]
async fn corrupt_replay_record_returns_500_without_panicking() {
    let (_fixture, server) = setup();
    server.db.last_block.set((), 30_001);
    server.db.block_info.set(
        30_001,
        BlockInfo {
            hash: BlockHash::from_byte_array([2; 32]),
            created: 2,
        },
    );
    server
        .db
        .storage
        .db
        .put_cf(&server.db.sync_blocks.cf(), SyncHeight::get_bytes(&30_001), [255, 255])
        .unwrap();
    let hash = BlockHash::from_byte_array([1; 32]);
    assert_eq!(request(server, "GET", &format!("/sync/blocks?after_height=30000&after_hash={hash}")).await.0, 500);
}

#[test]
fn identities_preserve_networks_and_allow_future_chain_strings() {
    assert_eq!(
        crate::sync::identity(Blockchain::Litecoin, Network::Regtest).unwrap(),
        ("litecoin".into(), "regtest".into())
    );
    assert_eq!(crate::sync::identity(Blockchain::Dogecoin, Network::Signet).unwrap().1, "signet");
    let status: nintondo_holdings_protocol::SyncStatus = serde_json::from_value(serde_json::json!({
        "protocol_version":2, "chain":"futurechain", "network":"future-network", "tip":{"height":1,"hash":"abc"},
        "replay_start":1,"catching_up":false
    }))
    .unwrap();
    assert_eq!(status.chain, "futurechain");
}
