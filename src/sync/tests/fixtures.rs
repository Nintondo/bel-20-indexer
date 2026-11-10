use crate::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use tower::ServiceExt;

pub(super) struct Fixture {
    _dir: tempfile::TempDir,
    _events: kanal::Receiver<crate::server::RawServerEvent>,
}

pub(super) fn setup() -> (Fixture, Arc<Server>) {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        std::env::set_var("BLOCKCHAIN", "bells");
        std::env::set_var("RPC_URL", "http://127.0.0.1:1");
        std::env::set_var("RPC_USER", "test");
        std::env::set_var("RPC_PASS", "test");
        std::env::remove_var("NETWORK");
        std::env::set_var("SYNC_TOKEN", "test-sync-token-with-at-least-32-characters");
    });
    let temp = tempfile::tempdir().unwrap();
    let (events, _, server) = Server::new(temp.path().to_str().unwrap()).unwrap();
    let server = Arc::new(server);
    server.catching_up.store(false, std::sync::atomic::Ordering::Release);
    server.db.last_block.set((), 30_000);
    server.db.sync_state.set((), crate::sync::storage::ReplayState::V1 { start: 30_001 });
    server.db.proof_of_history.set(30_000, sha256::Hash::from_byte_array([3; 32]));
    server.db.block_info.set(
        30_000,
        BlockInfo {
            hash: BlockHash::from_byte_array([1; 32]),
            created: 1,
        },
    );
    (Fixture { _dir: temp, _events: events }, server)
}

pub(super) fn seed(server: &Server, byte: u8, tick: [u8; 4], amount: u64) -> AddressToken {
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

pub(super) fn empty_block(prev: [u8; 32], nonce: u32) -> nint_blk::proto::block::Block {
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

pub(super) async fn request(server: Arc<Server>, method: &str, path: &str) -> (axum::http::StatusCode, Vec<u8>) {
    let app = Router::new()
        .nest("/sync", crate::sync::routes::router(server.clone()).finish_api(&mut Default::default()))
        .merge(crate::rest::health_routes())
        .with_state(server);
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", "Bearer test-sync-token-with-at-least-32-characters")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, to_bytes(response.into_body(), 1024 * 1024).await.unwrap().to_vec())
}
