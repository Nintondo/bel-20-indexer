//! Versioned wire contract shared by the indexers and the wallet overview service.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    Bells,
    Doge,
    Pepe,
}

impl Chain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bells => "bells",
            Self::Doge => "doge",
            Self::Pepe => "pepe",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BlockRef {
    pub height: u32,
    pub hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncStatus {
    pub protocol_version: u32,
    pub chain: Chain,
    pub network: Network,
    pub tip: BlockRef,
}

/// Absolute post-block values. A zero balance removes a holding.
/// Amounts are nonnegative decimal strings with at most 18 fractional digits.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Balance {
    pub address: String,
    pub tick: String,
    pub token_id: String,
    pub decimals: u8,
    pub available: String,
    pub transferable: String,
    pub transfers_count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncBlock {
    pub block: BlockRef,
    pub prev_hash: String,
    pub created: u32,
    pub balances: Vec<Balance>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlockPage {
    pub status: SyncStatus,
    pub blocks: Vec<SyncBlock>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotCreated {
    pub id: String,
    pub status: SyncStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotPage {
    pub items: Vec<Balance>,
    pub next_cursor: Option<String>,
}
