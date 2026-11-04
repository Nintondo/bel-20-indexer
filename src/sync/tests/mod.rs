mod errors;
mod fixtures;
mod lifecycle;
mod safety;
use crate::*;
use fixtures::*;
use nintondo_holdings_protocol::{BlockPage, SnapshotCreated, SnapshotPage};

#[tokio::test]
async fn snapshot_pages_are_immutable_and_explicitly_closed() {
    let (_temp, server) = setup();
    let first = seed(&server, 1, *b"AAAA", 10);
    seed(&server, 2, *b"BBBB", 20);
    let (status, body) = request(server.clone(), "POST", "/sync/snapshot").await;
    assert_eq!(status, 200);
    let snapshot: SnapshotCreated = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.status.tip.height, 30_000);
    server.db.address_token_to_balance.set(
        first,
        TokenBalance {
            balance: 999.into(),
            ..Default::default()
        },
    );
    seed(&server, 3, *b"CCCC", 30);
    let (_, body) = request(server.clone(), "GET", &format!("/sync/snapshot/{}?limit=1", snapshot.id)).await;
    let first: SnapshotPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(first.items[0].available, "10");
    let (_, body) = request(
        server.clone(),
        "GET",
        &format!("/sync/snapshot/{}?limit=1&cursor={}", snapshot.id, first.next_cursor.unwrap()),
    )
    .await;
    let second: SnapshotPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(second.items[0].available, "20");
    assert!(second.next_cursor.is_none());
    assert_eq!(request(server.clone(), "DELETE", &format!("/sync/snapshot/{}", snapshot.id)).await.0, 204);
    assert_eq!(request(server.clone(), "GET", &format!("/sync/snapshot/{}", snapshot.id)).await.0, 410);
}

#[tokio::test]
async fn committed_empty_blocks_replay_and_reorg_removes_orphans() {
    let (_temp, server) = setup();
    let cache = Arc::new(parking_lot::Mutex::new(ReorgCache::new()));
    let indexer = crate::inscriptions::indexer::InscriptionIndexer::new(server.clone(), cache.clone());
    let block = empty_block([1; 32], 1);
    let old_hash = block.header.hash.to_string();
    indexer.handle(30_001, block, true).unwrap();
    server.catching_up.store(false, std::sync::atomic::Ordering::Release);
    let parent = BlockHash::from_byte_array([1; 32]).to_string();
    let (_, body) = request(server.clone(), "GET", &format!("/sync/blocks?after_height=30000&after_hash={parent}")).await;
    let page: BlockPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.blocks.len(), 1);
    assert!(page.blocks[0].balances.is_empty());
    assert_eq!(page.blocks[0].prev_hash, parent);
    cache.lock().restore(&server, 30_000).unwrap();
    assert!(server.db.sync_blocks.try_get(30_001).unwrap().is_none());
    assert!(server.db.proof_of_history.get(30_001).is_none());
    indexer.handle(30_001, empty_block([1; 32], 2), true).unwrap();
    server.catching_up.store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(
        request(server.clone(), "GET", &format!("/sync/blocks?after_height=30001&after_hash={old_hash}")).await.0,
        409
    );
    server.db.block_info.set(
        29_999,
        BlockInfo {
            hash: BlockHash::from_byte_array([2; 32]),
            created: 0,
        },
    );
    let older = BlockHash::from_byte_array([2; 32]).to_string();
    assert_eq!(request(server, "GET", &format!("/sync/blocks?after_height=29999&after_hash={older}")).await.0, 410);
}
