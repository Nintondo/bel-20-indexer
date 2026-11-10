//! Versioned wire contract shared by the indexers and the wallet overview service.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 2;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BlockRef {
    pub height: u32,
    pub hash: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncStatus {
    pub protocol_version: u32,
    pub chain: String,
    pub network: String,
    pub tip: BlockRef,
    /// First replayable block; older checkpoints require a snapshot.
    pub replay_start: u32,
    /// Source is replaying historical blocks and cannot provide a live import yet.
    pub catching_up: bool,
}

/// Absolute post-block values. A zero balance removes a holding.
/// Amounts are nonnegative decimal strings with at most 18 fractional digits.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Balance {
    pub address: String,
    /// Full source script hash; multiple scripts can share an address.
    pub account_id: String,
    pub tick: String,
    pub token_id: String,
    pub decimals: u8,
    pub available: String,
    pub transferable: String,
    pub transfers_count: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncBlock {
    pub block: BlockRef,
    pub prev_hash: String,
    pub created: u32,
    pub balances: Vec<Balance>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlockPage {
    pub status: SyncStatus,
    pub blocks: Vec<SyncBlock>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotCreated {
    pub id: String,
    pub status: SyncStatus,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotPage {
    pub items: Vec<Balance>,
    pub next_cursor: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum SyncEvent {
    Ready,
    NewBlock { height: u32, blockhash: String },
    Reorg { blocks_count: u32, new_height: u32 },
}
