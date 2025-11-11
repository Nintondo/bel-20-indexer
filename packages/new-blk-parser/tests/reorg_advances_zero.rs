use std::sync::Arc;

use bellscoin::hashes::{sha256d, Hash};
use std::str::FromStr;

use nint_blk::{proto::{block::Block, header::BlockHeader, tx::RawTx, varuint::VarUint}, CoinType, BlockEvent, RpcRead};

mod support;
use support::mock_rpc::MockRpc;

fn mk_block(prev: sha256d::Hash) -> Block {
    // Minimal block: no txs; header prev_hash must link to last_sent_hash
    let header = BlockHeader { version: 0, prev_hash: prev, merkle_root: sha256d::Hash::all_zeros(), timestamp: 0, bits: 0, nonce: 0 };
    Block::new(80, header, None, VarUint::from(0u8), Vec::<RawTx>::new())
}

#[test]
fn emits_zero_reorg_len_when_advancing() {
    // Arrange a minimal chain: genesis (0), then heights 1 and 2.
    let g0 = sha256d::Hash::from_byte_array([0u8; 32]);
    let h1 = sha256d::Hash::from_byte_array([1u8; 32]);
    let h2 = sha256d::Hash::from_byte_array([2u8; 32]);

    let rpc = MockRpc::default().with_best(h2);
    rpc.set_height(0, g0);
    rpc.set_height(1, h1);
    rpc.set_height(2, h2);
    rpc.set_block(h1, mk_block(g0));
    // prev of block2 must match computed header.hash of block1
    let b1 = mk_block(g0);
    let b2 = mk_block(b1.header.hash);
    rpc.set_block(h1, b1);
    rpc.set_block(h2, b2);

    // checkpoint is valid (no reorg); best height is 2
    rpc.set_info(g0, nint_blk::GetBlockResult {
        hash: g0, confirmations: 100, size: 0, strippedsize: None, weight: 0, height: 0, version: 0, time: 0, mediantime: None, nonce: 0, bits: "".into(), difficulty: 0.0, previousblockhash: None, nextblockhash: None,
    });
    rpc.set_info(h2, nint_blk::GetBlockResult {
        hash: h2, confirmations: 0, size: 0, strippedsize: None, weight: 0, height: 2, version: 0, time: 0, mediantime: None, nonce: 0, bits: "".into(), difficulty: 0.0, previousblockhash: Some(h1), nextblockhash: None,
    });

    // Build indexer with empty on-disk chain, starting from height 0
    let indexer = Arc::new(nint_blk::Indexer {
        path: None,
        index_dir_path: None,
        coin: CoinType::from_str("bellscoin").unwrap(),
        token: dutils::wait_token::WaitToken::default(),
        last_block: nint_blk::BlockId { height: 0, hash: g0 },
        reorg_max_len: 8,
        client: Arc::new(rpc) as Arc<dyn RpcRead + Send + Sync>,
    });

    // Act: start stream and collect first two events
    let rx = indexer.clone().parse_blocks();
    let mut out = Vec::<BlockEvent>::new();
    for _ in 0..20 { // wait up to ~1s
        if let Ok(Some(ev)) = rx.try_recv() {
            out.push(ev);
            if out.len() == 2 { break; }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Stop background thread
    indexer.token.cancel();

    // Assert: got two advancing blocks with reorg_len == 0
    assert_eq!(out.len(), 2, "expected two block events");
    assert_eq!(out[0].id.height, 1);
    assert_eq!(out[1].id.height, 2);
    assert_eq!(out[0].reorg_len, 0);
    assert_eq!(out[1].reorg_len, 0);
}
