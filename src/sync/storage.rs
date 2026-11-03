//! On-disk schema. Keep V1 layouts immutable; add a new variant for migrations.
use crate::{db::*, Fixed128};
use anyhow::{ensure, Context};
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
}

impl ReplayState {
    pub fn start(&self) -> u32 {
        match self {
            Self::V1 { start } => *start,
        }
    }
}

pub fn initialize(db: &DB) -> anyhow::Result<()> {
    if db.sync_state.try_get(())?.is_none() {
        // Legacy SYNC_BLOCKS records used wire structs without a schema version.
        // This log is disposable: reset it without touching authoritative balances.
        db.storage.delete_range(&db.sync_blocks, &0, &u32::MAX)?;
        let start = db.last_block.try_get(())?.unwrap_or_default().saturating_add(1);
        let mut batch = RocksBatch::default();
        batch.remove(&db.sync_blocks, u32::MAX);
        batch.set(&db.sync_state, (), ReplayState::V1 { start });
        batch.commit(&db.storage)?;
    }
    Ok(())
}

/// A missing record within this boundary means no balances changed in that block.
pub fn append(db: &DB, batch: &mut RocksBatch, height: u32, changes: Vec<BalanceV1>, live: bool, retention: u32) -> anyhow::Result<()> {
    let state = db.sync_state.try_get(())?.context("Replay state missing")?;
    let start = state.start().max(height.saturating_sub(retention - 1));
    let start = if live { start } else { height.saturating_add(1) };
    batch.set(&db.sync_state, (), ReplayState::V1 { start });
    // Remove expired sparse entries. Normally at most one height expires per block.
    for key in db.sync_blocks.keys_between(&state.start(), &start)? {
        batch.remove(&db.sync_blocks, key);
    }
    if live && !changes.is_empty() {
        batch.set(&db.sync_blocks, height, Changes::V1(changes));
    }
    Ok(())
}

pub fn read(db: &DB, height: u32) -> anyhow::Result<nintondo_holdings_protocol::SyncBlock> {
    let state = db.sync_state.try_get(())?.context("Replay state missing")?;
    ensure!(height >= state.start(), "Block predates replay boundary");
    let info = db.block_info.try_get(height)?.context("Canonical block metadata missing")?;
    let prev = db.block_info.try_get(height.saturating_sub(1))?.context("Parent block metadata missing")?;
    let balances = match db.sync_blocks.try_get(height)? {
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
