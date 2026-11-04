//! On-disk schema. Keep V1 layouts immutable; add a new variant for migrations.
use crate::{db::*, Fixed128};
use anyhow::{ensure, Context};
use bitcoin_hashes::Hash;
use rocksdb_wrapper::RocksBatch;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub enum Changes {
    V1(Vec<BalanceV1>),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BalanceV1 {
    address: String,
    script_hash: [u8; 32],
    tick: String,
    genesis: String,
    decimals: u8,
    available: Fixed128,
    transferable: Fixed128,
    transfers: u64,
}

impl BalanceV1 {
    pub fn new(address: String, key: AddressToken, value: &TokenBalance, meta: &TokenMetaDB) -> Self {
        Self {
            address,
            script_hash: *key.address,
            tick: key.token.to_string(),
            genesis: meta.genesis.to_string(),
            decimals: meta.proto.dec,
            available: value.balance,
            transferable: value.transferable_balance,
            transfers: value.transfers_count,
        }
    }

    pub fn into_api(self) -> nintondo_holdings_protocol::Balance {
        nintondo_holdings_protocol::Balance {
            address: self.address,
            account_id: hex::encode(self.script_hash),
            tick: self.tick,
            token_id: self.genesis,
            decimals: self.decimals,
            available: self.available.to_string(),
            transferable: self.transferable.to_string(),
            transfers_count: self.transfers,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum ReplayState {
    V1 { start: u32 },
    V2 { start: u32, tip: Option<(u32, [u8; 32])> },
}

impl ReplayState {
    pub fn start(&self) -> u32 {
        match self {
            Self::V1 { start } | Self::V2 { start, .. } => *start,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum Identity {
    V1 { chain: String, network: String },
}
pub fn bind_identity(db: &DB, chain: String, network: String) -> anyhow::Result<()> {
    if let Some(Identity::V1 {
        chain: saved_chain,
        network: saved_network,
    }) = db.sync_identity.try_get(())?
    {
        ensure!((chain, network) == (saved_chain, saved_network), "Configured chain/network differs from this database");
    } else {
        let mut batch = RocksBatch::default();
        batch.set(&db.sync_identity, (), Identity::V1 { chain, network });
        batch.commit(&db.storage)?;
    }
    Ok(())
}

pub fn initialize(db: &DB) -> anyhow::Result<()> {
    let tip = db
        .last_block
        .try_get(())?
        .map(|height| -> anyhow::Result<_> {
            let info = db.block_info.try_get(height)?.context("Saved tip metadata missing")?;
            Ok((height, info.hash.to_byte_array()))
        })
        .transpose()?;
    let coherent = matches!(db.sync_state.try_get(()), Ok(Some(ReplayState::V2 { tip: saved, .. })) if saved == tip);
    if !coherent {
        // Legacy SYNC_BLOCKS records used wire structs without a schema version.
        // This log is disposable: reset it without touching authoritative balances.
        db.storage.delete_range(&db.sync_blocks, &0, &u32::MAX)?;
        let start = db.last_block.try_get(())?.unwrap_or_default().saturating_add(1);
        let mut batch = RocksBatch::default();
        batch.remove(&db.sync_blocks, u32::MAX);
        batch.set(&db.sync_state, (), ReplayState::V2 { start, tip });
        batch.commit(&db.storage)?;
    }
    db.storage.drop_disposable_table("SYNC_BLOCKS")?;
    db.storage.drop_disposable_table("SYNC_STATE")?;
    Ok(())
}

/// A missing record within this boundary means no balances changed in that block.
pub fn append(db: &DB, batch: &mut RocksBatch, height: u32, changes: Vec<BalanceV1>, live: bool, retention: u32) -> anyhow::Result<u32> {
    let state = db.sync_state.try_get(())?.context("Replay state missing")?;
    let start = state.start().max(height.saturating_sub(retention - 1));
    let start = if live { start } else { start.max(height.saturating_add(1)) };
    batch.set(&db.sync_state, (), ReplayState::V1 { start });
    // Also remove a stale record when replaying a now-empty canonical block.
    batch.remove(&db.sync_blocks, height);
    if live && height >= start && !changes.is_empty() {
        batch.set(&db.sync_blocks, height, Changes::V1(changes));
    }
    Ok(start)
}

/// Publish a gap in the same atomic batch as balances if export preparation fails.
pub fn invalidate(db: &DB, batch: &mut RocksBatch, height: u32, floor: u32) {
    batch.set(
        &db.sync_state,
        (),
        ReplayState::V1 {
            start: floor.max(height.saturating_add(1)),
        },
    );
    batch.remove(&db.sync_blocks, height);
}

/// Boundary commits first; range tombstones are written outside the publication gate.
pub fn prune(db: &DB) -> anyhow::Result<()> {
    let start = db.sync_state.try_get(())?.context("Replay state missing")?.start();
    db.storage.delete_range(&db.sync_blocks, &0, &start)
}

#[cfg(test)]
pub fn read(db: &DB, height: u32) -> anyhow::Result<nintondo_holdings_protocol::SyncBlock> {
    read_snapshot(db, &db.storage.snapshot(), height)
}

pub fn read_snapshot(db: &DB, snapshot: &rocksdb_wrapper::RocksSnapshot, height: u32) -> anyhow::Result<nintondo_holdings_protocol::SyncBlock> {
    let state = snapshot.get(&db.sync_state, ())?.context("Replay state missing")?;
    ensure!(height >= state.start(), "Block predates replay boundary");
    let info = snapshot.get(&db.block_info, height)?.context("Canonical block metadata missing")?;
    let prev = snapshot.get(&db.block_info, height.saturating_sub(1))?.context("Parent block metadata missing")?;
    let balances = match snapshot.get(&db.sync_blocks, height)? {
        Some(Changes::V1(items)) => items.into_iter().map(BalanceV1::into_api).collect(),
        None => vec![],
    };
    Ok(nintondo_holdings_protocol::SyncBlock {
        block: nintondo_holdings_protocol::BlockRef {
            height,
            hash: info.hash.to_string(),
        },
        prev_hash: prev.hash.to_string(),
        created: info.created,
        balances,
    })
}
