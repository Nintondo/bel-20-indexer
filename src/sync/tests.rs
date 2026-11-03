use crate::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use nintondo_holdings_protocol::{BlockPage, SnapshotCreated, SnapshotPage};
use tower::ServiceExt;

struct Fixture {
    _dir: tempfile::TempDir,
    _events: kanal::Receiver<crate::server::RawServerEvent>,
}

fn setup() -> (Fixture, Arc<Server>) {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        std::env::set_var("BLOCKCHAIN", "bells");
        std::env::set_var("RPC_URL", "http://127.0.0.1:1");
        std::env::set_var("RPC_USER", "test");
        std::env::set_var("RPC_PASS", "test");
        std::env::remove_var("NETWORK");
    });
    let temp = tempfile::tempdir().unwrap();
    let (events, _, server) = Server::new(temp.path().to_str().unwrap()).unwrap();
    let server = Arc::new(server);
    server.db.last_block.set((), 30_000);
    server.db.block_info.set(
        30_000,
        BlockInfo {
            hash: BlockHash::from_byte_array([1; 32]),
            created: 1,
        },
    );
    (Fixture { _dir: temp, _events: events }, server)
}

fn seed(server: &Server, byte: u8, tick: [u8; 4], amount: u64) -> AddressToken {
    let key = AddressToken {
        address: [byte; 32].into(),
        token: tick.into(),
    };
    server.db.fullhash_to_address.set(key.address, format!("Address{byte}"));
    server.db.token_to_meta.set(
        LowerCaseTokenTick::from(tick),
        TokenMetaDB {
            genesis: InscriptionId {
                txid: Txid::from_byte_array(tick.repeat(8).try_into().unwrap()),
                index: 0,
            },
            proto: DeployProtoDB {
                tick: tick.into(),
                max: amount.into(),
                lim: amount.into(),
                dec: 18,
                supply: amount.into(),
                transfer_count: 0,
                mint_count: 1,
                height: 30_000,
                created: 1,
                deployer: key.address,
                transactions: 1,
            },
        },
    );
    server.db.address_token_to_balance.set(
        key,
        TokenBalance {
            balance: amount.into(),
            ..Default::default()
        },
    );
    key
}

fn empty_block(prev: [u8; 32], nonce: u32) -> nint_blk::proto::block::Block {
    let header = nint_blk::proto::header::BlockHeader {
        version: 1,
        prev_hash: bellscoin::hashes::sha256d::Hash::from_byte_array(prev),
        merkle_root: bellscoin::hashes::sha256d::Hash::all_zeros(),
        timestamp: 2,
        bits: 0,
        nonce,
    };
    let coinbase = nint_blk::proto::tx::RawTx {
        version: 1,
        in_count: 1u8.into(),
        inputs: vec![nint_blk::proto::tx::TxInput {
            outpoint: OutPoint::null(),
            script_len: 0u8.into(),
            script_sig: vec![],
            seq_no: u32::MAX,
            witness: Default::default(),
        }],
        out_count: 0u8.into(),
        outputs: vec![],
        locktime: 0,
        coin: nint_blk::CoinType::from_str("bellscoin").unwrap(),
    };
    nint_blk::proto::block::Block::new(80, header, None, 1u8.into(), vec![coinbase])
}

async fn request(server: Arc<Server>, method: &str, path: &str) -> (axum::http::StatusCode, Vec<u8>) {
    let app = Router::new().nest("/sync", super::routes::router()).with_state(server);
    let response = app.oneshot(Request::builder().method(method).uri(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    (status, to_bytes(response.into_body(), 1024 * 1024).await.unwrap().to_vec())
}

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
    let parent = BlockHash::from_byte_array([1; 32]).to_string();
    let (_, body) = request(server.clone(), "GET", &format!("/sync/blocks?after_height=30000&after_hash={parent}")).await;
    let page: BlockPage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.blocks.len(), 1);
    assert!(page.blocks[0].balances.is_empty());
    assert_eq!(page.blocks[0].prev_hash, parent);
    cache.lock().restore(&server, 30_000).unwrap();
    assert!(server.db.sync_blocks.get(30_001).is_none());
    assert!(server.db.proof_of_history.get(30_001).is_none());
    indexer.handle(30_001, empty_block([1; 32], 2), true).unwrap();
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

#[test]
fn export_includes_zeroes_new_addresses_and_new_metadata() {
    let (_temp, server) = setup();
    let key = seed(&server, 1, *b"AAAA", 10);
    let processed = vec![crate::inscriptions::process_data::ProcessedData::Tokens {
        metas: vec![],
        balances: vec![(key, TokenBalance::default())],
        transfers_to_write: vec![],
        transfers_to_remove: vec![],
    }];
    let block = empty_block([1; 32], 1);
    let exported = super::export::block(&server.db, 30_001, &block.header, &processed).unwrap();
    assert_eq!(exported.balances.len(), 1);
    assert_eq!(exported.balances[0].available, "0");
    let mut new_meta = server.db.token_to_meta.get(LowerCaseTokenTick::from(*b"AAAA")).unwrap();
    new_meta.proto.tick = (*b"BBBB").into();
    let new_key = AddressToken {
        address: [9; 32].into(),
        token: (*b"BBBB").into(),
    };
    let processed = vec![
        crate::inscriptions::process_data::ProcessedData::FullHash {
            addresses: vec![(new_key.address, "NewAddress9".into())],
        },
        crate::inscriptions::process_data::ProcessedData::Tokens {
            metas: vec![(LowerCaseTokenTick::from(*b"BBBB"), new_meta)],
            balances: vec![(
                new_key,
                TokenBalance {
                    balance: 7.into(),
                    ..Default::default()
                },
            )],
            transfers_to_write: vec![],
            transfers_to_remove: vec![],
        },
    ];
    let exported = super::export::block(&server.db, 30_001, &block.header, &processed).unwrap();
    assert_eq!(exported.balances[0].address, "NewAddress9");
    assert_eq!(exported.balances[0].available, "7");
    assert_eq!(exported.balances[0].tick, "BBBB");
}
