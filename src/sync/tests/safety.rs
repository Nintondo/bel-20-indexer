use super::*;
use crate::{
    inscriptions::{
        commit::commit_block,
        indexer::{DataToWrite, InscriptionIndexer},
        process_data::ProcessedData,
    },
    sync::{config::*, storage},
};
use rocksdb_wrapper::{Pebble, RocksBatch};

#[test]
fn downtime_replay_is_independent_of_the_undo_window() {
    let (_fixture, server) = setup();
    let cache = Arc::new(parking_lot::Mutex::new(ReorgCache::new()));
    let indexer = InscriptionIndexer::new(server.clone(), cache.clone());
    let mut previous = [1; 32];
    // More than 30 blocks behind, but still within 10,000-block retention.
    for height in 30_001..=30_100 {
        let block = empty_block(previous, height);
        previous = block.header.hash.to_byte_array();
        indexer.handle(height, block, true).unwrap();
    }
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_001);
    assert_eq!(cache.lock().blocks.len(), 30);
    assert!(storage::read(&server.db, 30_001).unwrap().balances.is_empty());
    // A reorg during bulk processing still has undo data.
    cache.lock().restore(&server, 30_095).unwrap();
    assert_eq!(server.db.last_block.get(()), Some(30_095));
}

#[test]
fn disabled_replay_does_not_leave_records_below_boundary() {
    let (_fixture, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    let meta = server.db.token_to_meta.get(LowerCaseTokenTick::from(key.token)).unwrap();
    let changes = || {
        vec![storage::BalanceV1::new(
            "Address1".into(),
            key,
            &server.db.address_token_to_balance.get(key).unwrap(),
            &meta,
        )]
    };
    server.db.sync_state.set((), storage::ReplayState::V1 { start: 30_100 });
    let mut batch = RocksBatch::default();
    storage::append(&server.db, &mut batch, 30_090, changes(), true, REPLAY_RETENTION).unwrap();
    batch.commit(&server.db.storage).unwrap();
    assert!(server.db.sync_blocks.try_get(30_090).unwrap().is_none());
    // Leftover prototype records below the boundary are removed by range cleanup.
    server.db.sync_blocks.set(30_001, storage::Changes::V1(changes()));
    storage::prune(&server.db).unwrap();
    assert_eq!(server.db.sync_blocks.iter().count(), 0);
    // An empty replacement explicitly removes any stale record at that height.
    server.db.sync_blocks.set(30_100, storage::Changes::V1(changes()));
    let mut batch = RocksBatch::default();
    storage::append(&server.db, &mut batch, 30_100, vec![], true, REPLAY_RETENTION).unwrap();
    batch.commit(&server.db.storage).unwrap();
    assert!(server.db.sync_blocks.try_get(30_100).unwrap().is_none());
}

#[test]
fn export_failure_publishes_gap_without_stopping_balance_commit() {
    let (_fixture, server) = setup();
    server.db.storage.db.put_cf(&server.db.sync_state.cf(), SyncStateKey::get_bytes(&()), [255]).unwrap();
    let key = seed(&server, 1, *b"AAAA", 10);
    let data = DataToWrite {
        processed: vec![
            ProcessedData::Tokens {
                metas: vec![],
                balances: vec![(
                    key,
                    TokenBalance {
                        balance: 25.into(),
                        ..Default::default()
                    },
                )],
                transfers_to_write: vec![],
                transfers_to_remove: vec![],
            },
            ProcessedData::Info {
                block_number: 30_001,
                block_info: BlockInfo {
                    hash: BlockHash::from_byte_array([2; 32]),
                    created: 1,
                },
                block_proof: sha256::Hash::all_zeros(),
            },
        ],
        ..Default::default()
    };
    commit_block(&server, &mut ReorgCache::new(), 30_001, data, true).unwrap();
    assert_eq!(server.db.last_block.get(()), Some(30_001));
    assert_eq!(server.db.address_token_to_balance.get(key).unwrap().balance.to_string(), "25");
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_002);
    // Readiness read errors also fail closed without stopping the indexing loop.
    server.db.storage.db.put_cf(&server.db.block_info.cf(), 30_001u32.to_be_bytes(), [255]).unwrap();
    crate::inscriptions::commit::mark_synced(
        &server,
        nint_blk::BlockId {
            height: 30_001,
            hash: bellscoin::hashes::sha256d::Hash::from_byte_array([2; 32]),
        },
    )
    .unwrap();
    assert!(server.catching_up.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn snapshot_scans_are_bounded_and_cursors_advance_on_empty_pages() {
    let (_fixture, server) = setup();
    for index in 0..PAGE_SCAN_MAX + 2 {
        let mut address = [0; 32];
        address[..8].copy_from_slice(&(index as u64).to_be_bytes());
        server.db.address_token_to_balance.set(
            AddressToken {
                address: address.into(),
                token: (*b"AAAA").into(),
            },
            TokenBalance::default(),
        );
    }
    let created = server.snapshots.create(&server).unwrap();
    let first = server.snapshots.page(&server.db, &created.id, None, 10).unwrap();
    assert!(first.items.is_empty());
    let next = crate::sync::snapshots::decode_cursor(first.next_cursor.as_deref()).unwrap();
    assert!(next.is_some());
    let last = server.snapshots.page(&server.db, &created.id, next, 10).unwrap();
    assert!(last.items.is_empty());
    assert!(last.next_cursor.is_none());
}

#[tokio::test]
async fn workers_and_sse_reject_excess_requests() {
    let (_fixture, server) = setup();
    let permits = server.sync_limits.work.clone().try_acquire_many_owned(EXPORT_WORKERS as u32).unwrap();
    assert_eq!(request(server.clone(), "GET", "/sync/status").await.0, 429);
    assert_eq!(request(server.clone(), "POST", "/sync/snapshot").await.0, 429);
    drop(permits);
    assert_eq!(request(server.clone(), "GET", "/sync/status").await.0, 200);
    let _streams = server.sync_limits.streams.clone().try_acquire_many_owned(EVENT_CONNECTIONS as u32).unwrap();
    assert_eq!(request(server, "GET", "/sync/events").await.0, 429);
}

#[tokio::test]
async fn uppercase_hashes_and_snapshot_invalidation_are_supported() {
    let (_fixture, server) = setup();
    let hash = BlockHash::from_byte_array([171; 32]);
    server.db.block_info.set(30_000, BlockInfo { hash, created: 1 });
    assert_eq!(
        request(
            server.clone(),
            "GET",
            &format!("/sync/blocks?after_height=30000&after_hash={}", hash.to_string().to_uppercase())
        )
        .await
        .0,
        200
    );
    let created = server.snapshots.create(&server).unwrap();
    server.snapshots.clear();
    assert_eq!(request(server, "GET", &format!("/sync/snapshot/{}", created.id)).await.0, 410);
}

#[test]
fn database_identity_cannot_be_rebound_to_another_chain() {
    let (_fixture, server) = setup();
    assert!(storage::bind_identity(&server.db, "bells".into(), "mainnet".into()).is_ok());
    assert!(storage::bind_identity(&server.db, "doge".into(), "mainnet".into()).is_err());
    assert!(storage::bind_identity(&server.db, "bells".into(), "testnet".into()).is_err());
}

#[test]
fn failed_rollback_preserves_undo_for_retry() {
    let (_fixture, server) = setup();
    let cache = Arc::new(parking_lot::Mutex::new(ReorgCache::new()));
    InscriptionIndexer::new(server.clone(), cache.clone())
        .handle(30_001, empty_block([1; 32], 1), true)
        .unwrap();
    let snapshot = server.snapshots.create(&server);
    // The direct commit remains unready until a parser marker is verified.
    assert!(snapshot.is_err());
    server.catching_up.store(false, std::sync::atomic::Ordering::Release);
    let snapshot = server.snapshots.create(&server).unwrap();
    let mut cache = cache.lock();
    assert!(cache.restore_using(&server, 30_000, |_| anyhow::bail!("Injected write failure")).is_err());
    assert!(server.catching_up.load(std::sync::atomic::Ordering::Acquire));
    assert!(server.snapshots.page(&server.db, &snapshot.id, None, 10).is_err());
    assert!(cache.blocks.contains_key(&30_001));
    assert_eq!(server.db.last_block.get(()), Some(30_001));
    cache.restore(&server, 30_000).unwrap();
    assert!(cache.blocks.is_empty());
    assert_eq!(server.db.last_block.get(()), Some(30_000));
}

#[test]
fn binary_rollback_and_redeploy_cannot_silently_leave_a_replay_gap() {
    let (_fixture, server) = setup();
    let cache = Arc::new(parking_lot::Mutex::new(ReorgCache::new()));
    InscriptionIndexer::new(server.clone(), cache).handle(30_001, empty_block([1; 32], 1), true).unwrap();
    storage::initialize(&server.db).unwrap();
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_001);
    // An older binary updates authoritative tables but ignores export metadata.
    server.db.block_info.set(
        30_002,
        BlockInfo {
            hash: BlockHash::from_byte_array([3; 32]),
            created: 3,
        },
    );
    server.db.last_block.set((), 30_002);
    storage::initialize(&server.db).unwrap();
    assert_eq!(server.db.sync_state.get(()).unwrap().start(), 30_003);
}

#[tokio::test]
async fn oversized_first_replay_block_forces_snapshot_instead_of_unbounded_response() {
    let (_fixture, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    let meta = server.db.token_to_meta.get(LowerCaseTokenTick::from(key.token)).unwrap();
    let value = server.db.address_token_to_balance.get(key).unwrap();
    server.db.sync_blocks.set(
        30_001,
        storage::Changes::V1(vec![storage::BalanceV1::new("x".repeat(REPLAY_PAGE_BYTES), key, &value, &meta)]),
    );
    server.db.last_block.set((), 30_001);
    server.db.block_info.set(
        30_001,
        BlockInfo {
            hash: BlockHash::from_byte_array([2; 32]),
            created: 2,
        },
    );
    let hash = BlockHash::from_byte_array([1; 32]);
    assert_eq!(request(server, "GET", &format!("/sync/blocks?after_height=30000&after_hash={hash}")).await.0, 410);
}

#[test]
fn prototype_column_families_are_removed_for_binary_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let db = DB::open(path);
        db.storage.db.create_cf("SYNC_BLOCKS", &Default::default()).unwrap();
        db.storage.db.create_cf("SYNC_STATE", &Default::default()).unwrap();
        db.last_block.set((), 30_000);
        db.block_info.set(
            30_000,
            BlockInfo {
                hash: BlockHash::from_byte_array([1; 32]),
                created: 1,
            },
        );
    }
    let db = DB::open(path);
    storage::initialize(&db).unwrap();
    assert!(db.storage.db.cf_handle("SYNC_BLOCKS").is_none());
    assert!(db.storage.db.cf_handle("SYNC_STATE").is_none());
    assert_eq!(db.sync_state.cf, "default");
    assert_eq!(db.sync_blocks.cf, "default");
    assert_eq!(db.sync_identity.cf, "default");
}

#[tokio::test]
async fn canonical_lookup_fails_closed_even_below_an_unverified_tip() {
    let (_fixture, server) = setup();
    assert_eq!(request(server.clone(), "GET", "/sync/block/30000").await.0, 200);
    server.catching_up.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(request(server.clone(), "GET", "/sync/block/30000").await.0, 503);
    assert_eq!(request(server, "POST", "/sync/snapshot").await.0, 503);
}
