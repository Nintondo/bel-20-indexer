use super::*;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod indexer;
mod leaked;
mod parser;
mod process_data;
mod searcher;
pub mod structs;
mod tag;
mod utils;

use envelope::{ParsedEnvelope, RawEnvelope};
use indexer::InscriptionIndexer;
use nint_blk::BlockEvent;
use parser::Parser;
use process_data::ProcessedData;
use structs::Inscription;
use tag::Tag;

pub use structs::Location;

pub struct Indexer {
    server: Arc<Server>,
    reorg_cache: Arc<parking_lot::Mutex<ReorgCache>>,
}

impl Indexer {
    pub fn new(server: Arc<Server>) -> Self {
        Self {
            reorg_cache: Arc::new(parking_lot::Mutex::new(ReorgCache::new())),
            server,
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
        let res = self.index();

        self.reorg_cache.lock().restore_all(&self.server).track().ok();
        self.server.db.flush_all();

        res
    }

    fn index(&self) -> anyhow::Result<()> {
        let rx = self.server.indexer.clone().parse_blocks();

        let indexer = InscriptionIndexer::new(self.server.clone(), self.reorg_cache.clone());

        let mut progress: Option<Progress> = Some(Progress::begin("Indexing", self.server.indexer.last_block.height, self.server.indexer.last_block.height));

        let mut prev_height: Option<u64> = None;
        while !self.server.token.is_cancelled() {
            let data = match rx.try_recv() {
                Ok(Some(data)) => data,
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => break,
            };
            self.handle_event(&indexer, &mut progress, &mut prev_height, data)?;
            if self.server.token.is_cancelled() { return Ok(()); }
        }

        Ok(())
    }

    fn handle_event(
        &self,
        indexer: &InscriptionIndexer,
        progress: &mut Option<Progress>,
        prev_height: &mut Option<u64>,
        data: BlockEvent,
    ) -> anyhow::Result<()> {
        if let Some(p) = progress.as_mut() {
            p.update_len(data.tip.saturating_sub(REORG_CACHE_MAX_LEN as u64));
        }

        let BlockEvent { block, id, tip, reorg_len } = data;

        let handle_reorgs = id.height > tip - REORG_CACHE_MAX_LEN as u64;

        if handle_reorgs { progress.take(); }

        {
            let mut cache = self.reorg_cache.lock();
            if !cache.blocks.is_empty() && !handle_reorgs { cache.blocks.clear(); }
        }

        if reorg_len > 0 {
            warn!("Reorg detected: {} blocks", reorg_len);
            let restore_height = prev_height.unwrap_or_default().saturating_sub(reorg_len as u64);
            self.reorg_cache.lock().restore(&self.server, restore_height as u32)?;
            self.server.event_sender.send(ServerEvent::Reorg(reorg_len as u32, id.height as u32)).ok();
        }

        if let Some(last_reorg_height) = self.reorg_cache.lock().blocks.last_key_value().map(|x| x.0) {
            if last_reorg_height + 1 != id.height as u32 {
                anyhow::bail!("Wrong reorg cache tip height. Expected {}, got {}", last_reorg_height + 1, id.height as u32);
            }
        }

        indexer.handle(id.height as u32, block, handle_reorgs).track()?;
        *prev_height = Some(id.height);
        if let Some(p) = progress.as_ref() { p.inc(1); }
        Ok(())
    }

    #[cfg(test)]
    pub fn index_with(&self, rx: kanal::Receiver<nint_blk::BlockEvent>) -> anyhow::Result<()> {
        let indexer = InscriptionIndexer::new(self.server.clone(), self.reorg_cache.clone());
        let mut progress: Option<Progress> = Some(Progress::begin("Indexing", self.server.indexer.last_block.height, self.server.indexer.last_block.height));
        let mut prev_height: Option<u64> = None;

        loop {
            let data = match rx.try_recv() {
                Ok(Some(data)) => data,
                Ok(None) => { std::thread::sleep(Duration::from_millis(10)); continue; }
                Err(_) => break,
            };
            self.handle_event(&indexer, &mut progress, &mut prev_height, data)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bellscoin::hashes::{sha256d, Hash};
    use nint_blk::proto::{block::Block, header::BlockHeader, tx::RawTx, varuint::VarUint};

    #[test]
    fn index_with_empty_channel_returns_ok() {
        // Ensure required env vars exist before Server statics are accessed.
        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, _tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let indexer = Indexer::new(Arc::new(server));

        // Create a channel and immediately drop the sender to close it.
        let (tx, rx) = kanal::bounded::<nint_blk::BlockEvent>(1);
        drop(tx);

        // Should return Ok since the channel is closed and there are no events to process.
        indexer.index_with(rx).expect("index_with should return Ok");
    }

    fn mk_block(prev: sha256d::Hash, nonce: u32) -> Block {
        use std::str::FromStr;
        let header = BlockHeader { version: 0, prev_hash: prev, merkle_root: sha256d::Hash::all_zeros(), timestamp: 0, bits: 0, nonce };
        let raw = RawTx {
            version: 0,
            in_count: VarUint::from(0u8),
            inputs: vec![],
            out_count: VarUint::from(0u8),
            outputs: vec![],
            locktime: 0,
            coin: nint_blk::CoinType::from_str("bellscoin").unwrap(),
        };
        Block::new(80, header, None, VarUint::from(1u8), vec![raw])
    }

    #[test]
    fn reorg_len_zero_no_reorg_event_and_writes_info() {
        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);
        let indexer = Indexer::new(server.clone());

        let height = *START_HEIGHT + 1;
        let prev = sha256d::Hash::all_zeros();
        let block = mk_block(prev, 1);
        let bhash = block.header.hash;
        let id = nint_blk::BlockId { height: height as u64, hash: bhash };
        let tip = height as u64 + (REORG_CACHE_MAX_LEN as u64 - 1);
        let event = nint_blk::BlockEvent { id, block, reorg_len: 0, tip };

        let (sender, rx) = kanal::bounded::<nint_blk::BlockEvent>(2);
        sender.send(event).unwrap();
        drop(sender);

        // subscribe to events
        let mut erx = tx.subscribe();

        indexer.index_with(rx).expect("index_with");

        // no reorg event should be emitted
        let mut got_reorg = false;
        loop {
            match erx.try_recv() {
                Ok(ServerEvent::Reorg(_, _)) => { got_reorg = true; break; }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(_) => break,
            }
        }
        assert_eq!(got_reorg, false, "should not emit reorg when reorg_len == 0");

        let info = server.db.block_info.get(height).expect("block_info present");
        assert_eq!(info.hash, bhash.into());
    }

    #[test]
    fn reorg_positive_triggers_restore_and_updates_blockinfo() {
        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);
        let indexer = Indexer::new(server.clone());

        let height = *START_HEIGHT + 2;
        let prev = sha256d::Hash::all_zeros();
        let block1 = mk_block(prev, 7);
        let h1 = block1.header.hash;
        let id1 = nint_blk::BlockId { height: height as u64, hash: h1 };
        let tip = height as u64 + (REORG_CACHE_MAX_LEN as u64 - 1);
        let event1 = nint_blk::BlockEvent { id: id1, block: block1, reorg_len: 0, tip };

        let block2 = mk_block(prev, 42); // different nonce -> different hash
        let h2 = block2.header.hash;
        assert_ne!(h1, h2);
        let id2 = nint_blk::BlockId { height: height as u64, hash: h2 };
        let event2 = nint_blk::BlockEvent { id: id2, block: block2, reorg_len: 1, tip };

        let (sender, rx) = kanal::bounded::<nint_blk::BlockEvent>(2);
        sender.send(event1).unwrap();
        sender.send(event2).unwrap();
        drop(sender);

        let mut erx = tx.subscribe();

        indexer.index_with(rx).expect("index_with");

        // Expect a reorg event
        let mut reorg: Option<(u32, u32)> = None;
        loop {
            match erx.try_recv() {
                Ok(ServerEvent::Reorg(cnt, new_height)) => { reorg = Some((cnt, new_height)); break; }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(_) => break,
            }
        }
        let r = reorg.expect("reorg event expected");
        assert_eq!(r.0, 1);
        assert_eq!(r.1, height);

        // block_info at height should now reflect the second block's hash
        let info = server.db.block_info.get(height).expect("block_info present");
        assert_eq!(info.hash, h2.into());
    }

    #[test]
    fn partial_visibility_block_events_without_history_values() {
        use crate::db::{AddressTokenIdDB, HistoryValue, TokenHistoryDB};
        use crate::tokens::OriginalTokenTick;
        use crate::utils::FullHash;
        use bellscoin::Txid;

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, _tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);

        let height = *START_HEIGHT + 4;
        let tick = OriginalTokenTick::from_str("TICK").unwrap();
        let key = AddressTokenIdDB { address: FullHash::ZERO, token: tick, id: 1 };
        let hv = HistoryValue { height, action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 } };

        // Simulate the in-flight window by writing block_events first
        server.db.block_events.set(height, vec![key]);
        let keys = server.db.block_events.get(height).unwrap_or_default();
        assert_eq!(keys.len(), 1, "block_events keys must be present during window");

        // address_token_to_history should not have values yet
        let existing = server.db.address_token_to_history.multi_get_kv(keys.iter(), false);
        assert!(existing.is_empty(), "history rows should not be visible yet");

        // Now persist the history rows (completing the operation)
        server.db.address_token_to_history.extend(vec![(key, hv)]);
        let existing = server.db.address_token_to_history.multi_get_kv(keys.iter(), false);
        assert_eq!(existing.len(), 1, "history rows must be visible after write completes");
    }

    #[test]
    fn tokens_write_aborts_history_persisted_tokens_absent() {
        use crate::db::{AddressToken, TokenBalance, LowerCaseTokenTick, AddressTokenIdDB, HistoryValue, TokenHistoryDB};
        use crate::tokens::OriginalTokenTick;
        use crate::utils::FullHash;
        use bellscoin::Txid;

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, _tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);

        // Prepare token identifiers used for assertions (but do not write snapshots)
        let tick = OriginalTokenTick::from_str("TTTT").unwrap();
        let tick_lower = LowerCaseTokenTick::from(tick);
        let addr_token = AddressToken { address: FullHash::ZERO, token: tick };
        let _balance = TokenBalance::default();

        // Emulate: History was persisted for a block, but snapshots were not
        let height = *START_HEIGHT + 5;
        let key = AddressTokenIdDB { address: FullHash::ZERO, token: tick, id: 1 };
        let hv = HistoryValue { height, action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 } };
        server.db.block_events.set(height, vec![key]);
        server.db.address_token_to_history.extend(vec![(key, hv)]);

        // Ensure tokens CFs remain unchanged (not written)
        assert!(server.db.token_to_meta.get(&tick_lower).is_none(), "token meta should not be written");
        assert!(server.db.address_token_to_balance.get(&addr_token).is_none(), "balance should not be written");
    }

    #[test]
    fn poh_matches_snapshots_diverge_across_nodes() {
        use crate::db::{AddressToken, AddressTokenIdDB, HistoryValue, LowerCaseTokenTick, TokenBalance, TokenMetaDB, DeployProtoDB};
        use crate::tokens::{InscriptionId, OriginalTokenTick};
        use crate::utils::{AddressesFullHash, FullHash};
        use bellscoin::Txid;

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let t1 = tempfile::tempdir().expect("tempdir");
        let (_raw_rx_a, _tx_a, server_a0) = Server::new(t1.path().to_str().unwrap()).expect("server");
        let server_a = Arc::new(server_a0);

        let t2 = tempfile::tempdir().expect("tempdir");
        let (_raw_rx_b, _tx_b, server_b0) = Server::new(t2.path().to_str().unwrap()).expect("server");
        let server_b = Arc::new(server_b0);

        let height = *START_HEIGHT + 7;
        let tick = OriginalTokenTick::from_str("POHT").unwrap();
        let tick_lower: LowerCaseTokenTick = tick.into();

        // Prepare identical history entry on both nodes
        let key = AddressTokenIdDB { address: FullHash::ZERO, token: OriginalTokenTick::from_str("POHT").unwrap(), id: 1 };
        let hv = HistoryValue {
            height,
            action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 },
        };
        let history = vec![(key, hv)];

        // Compute identical PoH for both nodes
        let prev = *DEFAULT_HASH;
        let addresses = AddressesFullHash::new(std::collections::HashMap::new());
        let poh_a = Server::generate_history_hash(prev, &history, &addresses).expect("poh a");
        let poh_b = Server::generate_history_hash(prev, &history, &addresses).expect("poh b");
        assert_eq!(poh_a, poh_b, "generated PoH must match across nodes");

        // Write history + PoH to both nodes
        let info_a = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
        ProcessedData::History { block_number: height, last_history_id: 1, history: history.clone() }.write(&server_a, None);
        ProcessedData::Info { block_number: height, block_info: info_a, block_proof: poh_a }.write(&server_a, None);

        let info_b = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
        ProcessedData::History { block_number: height, last_history_id: 1, history: history.clone() }.write(&server_b, None);
        ProcessedData::Info { block_number: height, block_info: info_b, block_proof: poh_b }.write(&server_b, None);

        // On server A only, write snapshots (token meta + balance)
        let meta = TokenMetaDB {
            genesis: InscriptionId { txid: Txid::all_zeros(), index: 0 },
            proto: DeployProtoDB {
                tick: OriginalTokenTick::from_str("POHT").unwrap(),
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
        let addr_token = AddressToken { address: FullHash::ZERO, token: OriginalTokenTick::from_str("POHT").unwrap() };
        let balance = TokenBalance::default();
        ProcessedData::Tokens {
            metas: vec![(tick_lower.clone(), meta)],
            balances: vec![(addr_token, balance)],
            transfers_to_write: vec![],
            transfers_to_remove: vec![],
        }.write(&server_a, None);

        // Assertions
        // PoH equal
        let a_poh = server_a.db.proof_of_history.get(height).expect("poh a present");
        let b_poh = server_b.db.proof_of_history.get(height).expect("poh b present");
        assert_eq!(a_poh, b_poh, "PoH must match across nodes");

        // History equal
        assert_eq!(server_a.db.block_events.get(height), server_b.db.block_events.get(height));
        let events = server_a.db.block_events.get(height).unwrap();
        let a_hist = server_a.db.address_token_to_history.multi_get_kv(events.iter(), true);
        let b_hist = server_b.db.address_token_to_history.multi_get_kv(events.iter(), true);
        let a_keys: Vec<_> = a_hist.iter().map(|(k, _)| **k).collect();
        let b_keys: Vec<_> = b_hist.iter().map(|(k, _)| **k).collect();
        assert_eq!(a_keys, b_keys, "History keys must match across nodes");

        // Snapshots diverge
        assert!(server_a.db.token_to_meta.get(&tick_lower).is_some(), "A must have token meta");
        assert!(server_b.db.token_to_meta.get(&tick_lower).is_none(), "B must not have token meta");
    }

    #[test]
    fn same_height_replacement_reorglen_zero_causes_snapshot_divergence() {
        // This simulates a same-height replacement where the watcher fails to signal a reorg
        // (reorg_len = 0). Node A first processes block A at height H (writes History + Tokens),
        // then processes replacement block B at the same height H (writes History + Tokens again)
        // without rollback. Node B only processes B. Both produce identical PoH from B's history,
        // but Node A's snapshots carry residuals from block A, so snapshots diverge.

        use crate::db::{AddressToken, AddressTokenIdDB, HistoryValue, LowerCaseTokenTick, TokenBalance, TokenMetaDB, DeployProtoDB};
        use crate::tokens::{InscriptionId, OriginalTokenTick};
        use crate::utils::{AddressesFullHash, FullHash};
        use bellscoin::Txid;

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let t1 = tempfile::tempdir().expect("tempdir");
        let (_raw_rx_a, _tx_a, server_a0) = Server::new(t1.path().to_str().unwrap()).expect("server");
        let server_a = Arc::new(server_a0);

        let t2 = tempfile::tempdir().expect("tempdir");
        let (_raw_rx_b, _tx_b, server_b0) = Server::new(t2.path().to_str().unwrap()).expect("server");
        let server_b = Arc::new(server_b0);

        let height = *START_HEIGHT + 9;
        let tick = OriginalTokenTick::from_str("REOR").unwrap();
        let tick_lower: LowerCaseTokenTick = tick.into();

        // Block A: unique balance on Node A only
        let addr1 = AddressToken { address: FullHash::ZERO, token: OriginalTokenTick::from_str("REOR").unwrap() };
        let bal1 = TokenBalance::default();
        let meta = TokenMetaDB {
            genesis: InscriptionId { txid: Txid::all_zeros(), index: 0 },
            proto: DeployProtoDB {
                tick: OriginalTokenTick::from_str("REOR").unwrap(),
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
        // Emulate Node A processing block A at height H
        ProcessedData::Tokens {
            metas: vec![(tick_lower.clone(), meta.clone())],
            balances: vec![(addr1, bal1)],
            transfers_to_write: vec![],
            transfers_to_remove: vec![],
        }.write(&server_a, None);

        // Replacement Block B at the same height H; History and Tokens written on both nodes
        let key_b = AddressTokenIdDB { address: FullHash::ZERO, token: OriginalTokenTick::from_str("REOR").unwrap(), id: 1 };
        let hv_b = HistoryValue { height, action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 } };
        let history_b = vec![(key_b, hv_b)];
        let prev = *DEFAULT_HASH;
        let addresses = AddressesFullHash::new(std::collections::HashMap::new());
        let poh_b = Server::generate_history_hash(prev, &history_b, &addresses).expect("poh b");

        // Write history + proof (Info) to both nodes
        let info_b = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
        ProcessedData::History { block_number: height, last_history_id: 1, history: history_b.clone() }.write(&server_a, None);
        ProcessedData::Info { block_number: height, block_info: info_b, block_proof: poh_b }.write(&server_a, None);

        let info_b2 = BlockInfo { hash: bellscoin::BlockHash::all_zeros(), created: 0 };
        ProcessedData::History { block_number: height, last_history_id: 1, history: history_b.clone() }.write(&server_b, None);
        ProcessedData::Info { block_number: height, block_info: info_b2, block_proof: poh_b }.write(&server_b, None);

        // Tokens for B: identical on both nodes (a different address2), while A retains addr1 from A
        let addr2 = AddressToken { address: FullHash::from([1u8; 32]), token: OriginalTokenTick::from_str("REOR").unwrap() };
        let bal2 = TokenBalance::default();
        ProcessedData::Tokens {
            metas: vec![(tick_lower.clone(), meta.clone())],
            balances: vec![(addr2, bal2.clone())],
            transfers_to_write: vec![],
            transfers_to_remove: vec![],
        }.write(&server_a, None);
        ProcessedData::Tokens {
            metas: vec![(tick_lower.clone(), meta)],
            balances: vec![(addr2, bal2)],
            transfers_to_write: vec![],
            transfers_to_remove: vec![],
        }.write(&server_b, None);

        // Assertions
        // PoH equal after B
        let a_poh = server_a.db.proof_of_history.get(height).expect("poh a present");
        let b_poh = server_b.db.proof_of_history.get(height).expect("poh b present");
        assert_eq!(a_poh, b_poh, "PoH must match after replacement when history is identical");

        // History by height uses block_events -> identical
        assert_eq!(server_a.db.block_events.get(height), server_b.db.block_events.get(height));
        let events = server_a.db.block_events.get(height).unwrap();
        let a_hist = server_a.db.address_token_to_history.multi_get_kv(events.iter(), true);
        let b_hist = server_b.db.address_token_to_history.multi_get_kv(events.iter(), true);
        let a_keys: Vec<_> = a_hist.iter().map(|(k, _)| **k).collect();
        let b_keys: Vec<_> = b_hist.iter().map(|(k, _)| **k).collect();
        assert_eq!(a_keys, b_keys, "History keys must match after replacement");

        // Snapshot divergence: Node A has addr1 balance from A; Node B does not
        assert!(server_a.db.address_token_to_balance.get(&addr1).is_some(), "A retains balance from block A");
        assert!(server_b.db.address_token_to_balance.get(&addr1).is_none(), "B never saw block A snapshot");
    }

    #[test]
    fn logs_syncing_block_only_near_tip() {
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone)]
        struct TestMakeWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        struct TestWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl<'a> MakeWriter<'a> for TestMakeWriter {
            type Writer = TestWriter;
            fn make_writer(&'a self) -> Self::Writer { TestWriter(self.0.clone()) }
        }
        impl std::io::Write for TestWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let mut g = self.0.lock().unwrap();
                g.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let make = TestMakeWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt().with_writer(make).with_max_level(tracing::Level::DEBUG).finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, _tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);
        let indexer = Indexer::new(server);

        let height = *START_HEIGHT + 12;
        let prev = sha256d::Hash::all_zeros();
        let block = mk_block(prev, 5);
        let bhash = block.header.hash;
        let id = nint_blk::BlockId { height: height as u64, hash: bhash };
        let tip_near = height as u64 + (REORG_CACHE_MAX_LEN as u64 - 1);
        let ev = nint_blk::BlockEvent { id, block, reorg_len: 0, tip: tip_near };

        let (tx, rx) = kanal::bounded(1);
        tx.send(ev).unwrap();
        drop(tx);
        indexer.index_with(rx).expect("index_with");

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(output.contains("Syncing block:"), "should log syncing near tip; got: {output}");

        // Now test far behind: handle_reorgs should be false
        let buf2 = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let make2 = TestMakeWriter(buf2.clone());
        let subscriber2 = tracing_subscriber::fmt().with_writer(make2).with_max_level(tracing::Level::DEBUG).finish();
        let _guard2 = tracing::subscriber::set_default(subscriber2);

        let tmp2 = tempfile::tempdir().expect("tempdir");
        let (_raw_rx2, _tx2, server2) = Server::new(tmp2.path().to_str().unwrap()).expect("server");
        let server2 = Arc::new(server2);
        let indexer2 = Indexer::new(server2);

        let height2 = *START_HEIGHT + 13;
        let block2 = mk_block(sha256d::Hash::all_zeros(), 6);
        let id2 = nint_blk::BlockId { height: height2 as u64, hash: block2.header.hash };
        // tip one higher than near-tip threshold makes handle_reorgs false
        let tip_far = height2 as u64 + (REORG_CACHE_MAX_LEN as u64);
        let ev2 = nint_blk::BlockEvent { id: id2, block: block2, reorg_len: 0, tip: tip_far };
        let (tx2, rx2) = kanal::bounded(1);
        tx2.send(ev2).unwrap();
        drop(tx2);
        indexer2.index_with(rx2).expect("index_with");

        let binding2 = buf2.lock().unwrap();
        let out2 = String::from_utf8_lossy(&binding2);
        assert!(!out2.contains("Syncing block:"), "should not log syncing when far behind; got: {out2}");
    }

    #[test]
    fn logs_reorg_detected_on_positive_reorg() {
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone)]
        struct TestMakeWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        struct TestWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl<'a> MakeWriter<'a> for TestMakeWriter {
            type Writer = TestWriter;
            fn make_writer(&'a self) -> Self::Writer { TestWriter(self.0.clone()) }
        }
        impl std::io::Write for TestWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { let mut g = self.0.lock().unwrap(); g.extend_from_slice(buf); Ok(buf.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }

        std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
        std::env::set_var("RPC_USER", "user");
        std::env::set_var("RPC_PASS", "pass");
        std::env::set_var("BLOCKCHAIN", "bellscoin");

        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let make = TestMakeWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt().with_writer(make).with_max_level(tracing::Level::DEBUG).finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let tmp = tempfile::tempdir().expect("tempdir");
        let (_raw_rx, _tx, server) = Server::new(tmp.path().to_str().unwrap()).expect("server");
        let server = Arc::new(server);
        let indexer = Indexer::new(server);

        let height = *START_HEIGHT + 14;
        let block = mk_block(sha256d::Hash::all_zeros(), 7);
        let id = nint_blk::BlockId { height: height as u64, hash: block.header.hash };
        let tip = height as u64 + (REORG_CACHE_MAX_LEN as u64 - 1);
        let ev = nint_blk::BlockEvent { id, block, reorg_len: 2, tip };
        let (tx, rx) = kanal::bounded(1);
        tx.send(ev).unwrap();
        drop(tx);
        indexer.index_with(rx).expect("index_with");

        let bind3 = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&bind3);
        assert!(output.contains("Reorg detected: 2 blocks"), "expected reorg log present; got: {output}");
    }
}
