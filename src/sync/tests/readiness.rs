use super::*;
use crate::inscriptions::{commit::mark_synced, indexer::InscriptionIndexer};

#[tokio::test]
async fn readiness_waits_for_verified_tip_and_is_revoked_during_recovery() {
    let (_fixture, server) = setup();
    let saved = nint_blk::BlockId {
        height: 30_000,
        hash: bellscoin::hashes::sha256d::Hash::from_byte_array([1; 32]),
    };
    server.catching_up.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(request(server.clone(), "GET", "/livez").await.0, 200);
    assert_eq!(request(server.clone(), "GET", "/readyz").await.0, 503);
    mark_synced(&server, saved).unwrap();
    let (code, body) = request(server.clone(), "GET", "/readyz").await;
    assert_eq!(code, 200);
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["height"], 30_000);
    assert_eq!(status["blockhash"], "01".repeat(32));
    assert_eq!(status["proof"], "03".repeat(32));

    let cache = Arc::new(parking_lot::Mutex::new(ReorgCache::new()));
    let block = empty_block([1; 32], 1);
    let tip = nint_blk::BlockId {
        height: 30_001,
        hash: block.header.hash,
    };
    InscriptionIndexer::new(server.clone(), cache.clone()).handle(30_001, block, true).unwrap();
    assert_eq!(request(server.clone(), "GET", "/readyz").await.0, 503);
    mark_synced(&server, tip).unwrap();
    assert_eq!(request(server.clone(), "GET", "/readyz").await.0, 200);
    cache.lock().restore(&server, 30_000).unwrap();
    assert_eq!(request(server.clone(), "GET", "/readyz").await.0, 503);
    mark_synced(&server, saved).unwrap();
    assert_eq!(request(server.clone(), "GET", "/readyz").await.0, 200);
    server.token.cancel();
    assert_eq!(request(server, "GET", "/readyz").await.0, 503);
}

#[test]
fn idle_shutdown_restore_is_a_noop_but_real_rollback_still_requires_undo() {
    let (_fixture, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    let snapshot = server.snapshots.create(&server).unwrap();
    let mut cache = ReorgCache::new();
    cache.restore_all(&server).unwrap();
    assert_eq!(server.db.last_block.get(()), Some(30_000));
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "10");
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_001);
    assert!(server.snapshots.page(&server.db, &snapshot.id, None, 10).is_ok());
    assert!(!server.catching_up.load(std::sync::atomic::Ordering::Acquire));
    assert!(cache.restore(&server, 29_999).is_err());
    assert_eq!(server.db.last_block.get(()), Some(30_000));
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "10");
}
