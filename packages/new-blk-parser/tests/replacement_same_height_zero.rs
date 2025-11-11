use std::sync::Arc;

use bellscoin::hashes::{sha256d, Hash};
use std::str::FromStr;
use nint_blk::{proto::{block::Block, header::BlockHeader, tx::RawTx, varuint::VarUint}, CoinType, RpcRead};

mod support;
use support::mock_rpc::MockRpc;

fn mk_block(prev: sha256d::Hash) -> Block {
    let header = BlockHeader { version: 0, prev_hash: prev, merkle_root: sha256d::Hash::all_zeros(), timestamp: 0, bits: 0, nonce: 0 };
    Block::new(80, header, None, VarUint::from(0u8), Vec::<RawTx>::new())
}

#[test]
fn same_height_replacement_emits_zero_reorg() {
    // Genesis and two alternative blocks for height 1.
    let g0 = sha256d::Hash::from_byte_array([0u8; 32]);
    let h1a = sha256d::Hash::from_byte_array([1u8; 32]);
    let h1b = sha256d::Hash::from_byte_array([3u8; 32]);

    let rpc = MockRpc::default().with_best(h1a);
    rpc.set_height(0, g0);
    rpc.set_height(1, h1a);
    rpc.set_block(h1a, mk_block(g0));

    rpc.set_info(g0, nint_blk::GetBlockResult {
        hash: g0, confirmations: 100, size: 0, strippedsize: None, weight: 0, height: 0, version: 0, time: 0, mediantime: None, nonce: 0, bits: "".into(), difficulty: 0.0, previousblockhash: None, nextblockhash: None,
    });
    rpc.set_info(h1a, nint_blk::GetBlockResult {
        hash: h1a, confirmations: 0, size: 0, strippedsize: None, weight: 0, height: 1, version: 0, time: 0, mediantime: None, nonce: 0, bits: "".into(), difficulty: 0.0, previousblockhash: Some(g0), nextblockhash: None,
    });

    // Start indexer and read first event at height 1
    let indexer = Arc::new(nint_blk::Indexer {
        path: None,
        index_dir_path: None,
        coin: CoinType::from_str("bellscoin").unwrap(),
        token: dutils::wait_token::WaitToken::default(),
        last_block: nint_blk::BlockId { height: 0, hash: g0 },
        reorg_max_len: 8,
        client: Arc::new(rpc.clone()) as Arc<dyn RpcRead + Send + Sync>,
    });
    let rx = indexer.clone().parse_blocks();
    let first = loop {
        if let Ok(Some(ev)) = rx.try_recv() { break ev; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(first.id.height, 1);
    assert_eq!(first.reorg_len, 0);

    // Now simulate a replacement before future emissions by flipping the hash at height 1.
    // Since the watcher only fetches the current winner, it will still emit reorg_len = 0
    // when restarted from the same last block.
    rpc.set_height(1, h1b);
    rpc.set_block(h1b, mk_block(g0));

    // Stop first watcher
    indexer.token.cancel();

    // Restart watcher from the same last checkpoint
    let indexer2 = Arc::new(nint_blk::Indexer {
        path: None,
        index_dir_path: None,
        coin: CoinType::from_str("bellscoin").unwrap(),
        token: dutils::wait_token::WaitToken::default(),
        last_block: nint_blk::BlockId { height: 0, hash: g0 },
        reorg_max_len: 8,
        client: Arc::new(rpc) as Arc<dyn RpcRead + Send + Sync>,
    });
    let rx2 = indexer2.clone().parse_blocks();
    let ev = loop {
        if let Ok(Some(ev)) = rx2.try_recv() { break ev; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    indexer2.token.cancel();

    assert_eq!(ev.id.height, 1);
    assert_eq!(ev.reorg_len, 0, "replacement at same height should not be signaled as reorg");
}
