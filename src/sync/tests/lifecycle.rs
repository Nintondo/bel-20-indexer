use super::*;
use crate::{
    inscriptions::{commit::commit_block, indexer::DataToWrite, process_data::ProcessedData},
    sync::storage::{self, BalanceV1, ReplayState},
};

fn commit_balance(server: &Server, cache: &mut ReorgCache, key: AddressToken, height: u32, hash: u8, amount: u64, live: bool) {
    let value = TokenBalance {
        balance: amount.into(),
        ..Default::default()
    };
    let meta = server.db.token_to_meta.get(LowerCaseTokenTick::from(key.token)).unwrap();
    let data = DataToWrite {
        processed: vec![
            ProcessedData::Tokens {
                metas: vec![],
                balances: vec![(key, value.clone())],
                transfers_to_write: vec![],
                transfers_to_remove: vec![],
            },
            ProcessedData::Info {
                block_number: height,
                block_info: BlockInfo {
                    hash: BlockHash::from_byte_array([hash; 32]),
                    created: height,
                },
                block_proof: sha256::Hash::all_zeros(),
            },
        ],
        sync_balances: vec![BalanceV1::new("Address1".into(), key, &value, &meta)],
        ..Default::default()
    };
    commit_block(server, cache, height, data, live).unwrap();
}

#[tokio::test]
async fn replayed_balance_and_snapshot_match_after_reorg_and_shutdown_restore() {
    let (_fixture, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    let mut cache = ReorgCache::new();
    commit_balance(&server, &mut cache, key, 30_001, 2, 20, true);
    let parent = BlockHash::from_byte_array([1; 32]);
    let (_, body) = request(server.clone(), "GET", &format!("/sync/blocks?after_height=30000&after_hash={parent}")).await;
    let page: BlockPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.blocks[0].balances[0].available, "20");
    cache.restore(&server, 30_000).unwrap();
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "10");
    commit_balance(&server, &mut cache, key, 30_001, 3, 30, true);
    assert_eq!(storage::read(&server.db, 30_001).unwrap().balances[0].available, "30");
    let created = server.snapshots.create(&server).unwrap();
    assert_eq!(server.snapshots.page(&server.db, &created.id, None, 10).unwrap().items[0].available, "30");
    cache.restore_all(&server).unwrap();
    assert_eq!(server.db.last_block.get(()), Some(30_000));
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "10");
    assert!(server.db.sync_blocks.try_get(30_001).unwrap().is_none());
    // The consumer checkpoint is still valid until the source proves otherwise.
    let hash = BlockHash::from_byte_array([3; 32]);
    let (status, body) = request(server.clone(), "GET", &format!("/sync/blocks?after_height=30001&after_hash={hash}")).await;
    assert_eq!(status, 200);
    assert!(serde_json::from_slice::<BlockPage>(&body).unwrap().blocks.is_empty());
    assert_eq!(request(server.clone(), "GET", "/sync/block/30001").await.0, 503);
    commit_balance(&server, &mut cache, key, 30_001, 3, 30, true);
    assert_eq!(request(server, "GET", &format!("/sync/blocks?after_height=30001&after_hash={hash}")).await.0, 200);
}

#[tokio::test]
async fn sparse_retention_is_bounded_and_history_gaps_are_explicit() {
    let (_fixture, mut server) = setup();
    Arc::get_mut(&mut server).unwrap().sync_config.retention = 64;
    let key = seed(&server, 1, *b"AAAA", 10);
    let mut cache = ReorgCache::new();
    for height in 30_001..=30_080 {
        // Bulk indexing advances the boundary without creating replay records.
        commit_balance(&server, &mut cache, key, height, 2, 10, false);
    }
    assert_eq!(server.db.sync_blocks.iter().count(), 0);
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_081);
    for height in 30_081..=30_160 {
        commit_balance(&server, &mut cache, key, height, 2, 20, true);
    }
    assert_eq!(server.db.sync_blocks.iter().count(), 64);
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_097);
    let hash = BlockHash::from_byte_array([2; 32]);
    assert_eq!(request(server.clone(), "GET", &format!("/sync/blocks?after_height=30081&after_hash={hash}")).await.0, 410);
    assert_eq!(request(server.clone(), "GET", &format!("/sync/blocks?after_height=30096&after_hash={hash}")).await.0, 200);
    // A missing sparse record inside the retained interval is an empty block.
    server.db.sync_blocks.remove(30_100);
    assert!(storage::read(&server.db, 30_100).unwrap().balances.is_empty());
    server.db.sync_state.set((), ReplayState::V1 { start: 30_161 });
}

#[test]
fn stored_schema_is_versioned_and_legacy_log_reset_preserves_balances() {
    let (_fixture, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    server.db.storage.db.put_cf(&server.db.sync_blocks.cf(), 1u32.to_be_bytes(), [255]).unwrap();
    server.db.sync_state.remove(());
    storage::initialize(&server.db).unwrap();
    assert!(server.db.sync_blocks.try_get(1).unwrap().is_none());
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "10");
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_001);
}

#[test]
fn idle_sync_marker_recovers_readiness_only_at_the_committed_tip() {
    let (_fixture, server) = setup();
    server.catching_up.store(true, std::sync::atomic::Ordering::Release);
    let tip = nint_blk::BlockId {
        height: 30_000,
        hash: bellscoin::hashes::sha256d::Hash::from_byte_array([2; 32]),
    };
    crate::inscriptions::commit::mark_synced(&server, tip).unwrap();
    assert!(server.catching_up.load(std::sync::atomic::Ordering::Acquire));
    let tip = nint_blk::BlockId {
        height: 30_000,
        hash: bellscoin::hashes::sha256d::Hash::from_byte_array([1; 32]),
    };
    crate::inscriptions::commit::mark_synced(&server, tip).unwrap();
    assert!(!server.catching_up.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn snapshot_keeps_distinct_script_accounts_for_the_same_address() {
    let (_fixture, server) = setup();
    seed(&server, 1, *b"AAAA", 10);
    let other = seed(&server, 2, *b"AAAA", 20);
    server.db.fullhash_to_address.set(other.address, "Address1".to_owned());
    let created = server.snapshots.create(&server).unwrap();
    let page = server.snapshots.page(&server.db, &created.id, None, 10).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].address, page.items[1].address);
    assert_ne!(page.items[0].account_id, page.items[1].account_id);
}
