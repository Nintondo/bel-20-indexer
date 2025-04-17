use std::fmt::Display;
use std::str::FromStr;
use dutils::error::ContextWrapper;
use nintondo_dogecoin::hashes::Hash;
use nintondo_dogecoin::{OutPoint, Txid};
use serde::{Deserialize, Serialize};
use crate::types::token_history::HistoryLocation;

#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq, Eq, core::hash::Hash, Ord, PartialOrd, Copy,
)]
pub struct Location {
    pub outpoint: OutPoint,
    pub offset: u64,
}

impl Location {
    pub fn zero() -> Self {
        Self {
            offset: 0,
            outpoint: OutPoint {
                txid: Txid::all_zeros(),
                vout: 0,
            },
        }
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}i{}i{}",
            self.outpoint.txid, self.outpoint.vout, self.offset
        ))
    }
}

impl FromStr for Location {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let mut items = s.split(':');

        let error_msg = "Invalid location";

        let txid =
            Txid::from_str(items.next().anyhow_with(error_msg)?).anyhow_with("Invalid txid")?;
        let vout: u32 = items
            .next()
            .anyhow_with(error_msg)?
            .parse()
            .anyhow_with("Invalid vout")?;
        let offset: u64 = items
            .next()
            .anyhow_with(error_msg)?
            .parse()
            .anyhow_with("Invalid offset")?;

        Ok(Self {
            offset,
            outpoint: OutPoint { txid, vout },
        })
    }
}

impl From<HistoryLocation> for Location {
    fn from(value: HistoryLocation) -> Self {
        Location {
            outpoint: value.outpoint.into(),
            offset: value.offset,
        }
    }
}
