use crate::tokens::{FullHash, TokenTick};
use crate::Fixed128;
use dutils::error::ContextWrapper;
use electrs_client::{Fetchable, UpdateCapable};
use itertools::Itertools;
use nintondo_dogecoin::{Address, BlockHash, OutPoint, ScriptBuf};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TokenHistoryData {
    pub block_info: BlockInfo,
    pub inscriptions: Vec<TokenHistory>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ParsedTokenHistoryData {
    pub block_info: BlockInfo,
    pub inscriptions: Vec<ParsedTokenHistory>,
}

impl TryFrom<TokenHistoryData> for ParsedTokenHistoryData {
    type Error = TokenHistoryError;

    fn try_from(
        TokenHistoryData {
            block_info,
            inscriptions,
        }: TokenHistoryData,
    ) -> Result<Self, Self::Error> {
        let converted = inscriptions
            .into_iter()
            .map(|x| x.try_into())
            .collect::<Result<Vec<_>, _>>();
        Ok(Self {
            block_info,
            inscriptions: converted?,
        })
    }
}

impl UpdateCapable for TokenHistoryData {
    fn get_hash(&self) -> nintypes::common::hash::Hash256 {
        self.block_info.block_hash.into()
    }

    fn get_prev_hash(&self) -> nintypes::common::hash::Hash256 {
        self.block_info.prev_block_hash.into()
    }

    fn get_height(&self) -> electrs_client::BlockHeight {
        self.block_info.height
    }
}

impl From<InscriptionsTokenHistory> for Vec<TokenHistoryData> {
    fn from(value: InscriptionsTokenHistory) -> Self {
        value
            .data
            .into_iter()
            .map(|(block_info, inscriptions)| TokenHistoryData {
                block_info,
                inscriptions,
            })
            .collect()
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum ParsedTokenAddress {
    Standard(ScriptBuf),
    NonStandard(FullHash),
}

impl TryFrom<TokenAddress> for ParsedTokenAddress {
    type Error = nintondo_dogecoin::address::Error;

    fn try_from(value: TokenAddress) -> Result<Self, Self::Error> {
        match value {
            TokenAddress::Standard(str) => {
                let d = Address::from_str(&str)?.payload.script_pubkey();

                Ok(ParsedTokenAddress::Standard(d))
            }
            TokenAddress::NonStandard(hash) => Ok(ParsedTokenAddress::NonStandard(hash)),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ParsedTokenHistory {
    pub from: ParsedTokenAddress,
    pub to: ParsedTokenAddress,
    pub from_location: HistoryLocation,
    pub to_location: HistoryLocation,
    pub leaked: bool,
    pub token: ParsedTokenActionRest,
}

impl TryFrom<TokenHistory> for ParsedTokenHistory {
    type Error = TokenHistoryError;

    fn try_from(value: TokenHistory) -> Result<Self, Self::Error> {
        let TokenHistory {
            from,
            to,
            from_location,
            to_location,
            leaked,
            token,
        } = value.clone();
        Ok(Self {
            from: from
                .clone()
                .try_into()
                .map_err(|e| TokenHistoryError::ParseAddress {
                    error: e,
                    address: from,
                })?,
            to: to
                .clone()
                .try_into()
                .map_err(|e| TokenHistoryError::ParseAddress {
                    error: e,
                    address: to,
                })?,
            from_location,
            to_location,
            leaked,
            token,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenAddress {
    Standard(String),
    NonStandard(FullHash),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TokenHistory {
    pub from: TokenAddress,
    pub to: TokenAddress,
    pub from_location: HistoryLocation,
    pub to_location: HistoryLocation,
    pub leaked: bool,
    pub token: ParsedTokenActionRest,
}

#[derive(thiserror::Error, Debug)]
pub enum TokenHistoryError {
    #[error("Failed to parse address({address:?}) from str, {error}")]
    ParseAddress {
        error: nintondo_dogecoin::address::Error,
        address: TokenAddress,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InscriptionsTokenHistory {
    pub data: Vec<(BlockInfo, Vec<TokenHistory>)>,
    pub reorg: Vec<u32>,
}

impl Fetchable for InscriptionsTokenHistory {
    fn get_type() -> &'static str {
        "token_history"
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Copy)]
pub struct BlockInfo {
    pub height: u32,
    pub created: u32,
    pub block_hash: BlockHash,
    pub prev_block_hash: BlockHash,
}

impl From<BlockInfo> for crate::tokens::BlockHeader {
    fn from(v: BlockInfo) -> Self {
        Self {
            number: v.height,
            hash: v.block_hash.into(),
            prev_hash: v.prev_block_hash.into(),
        }
    }
}

impl From<&BlockInfo> for crate::tokens::BlockHeader {
    fn from(v: &BlockInfo) -> Self {
        Self {
            number: v.height,
            hash: v.block_hash.into(),
            prev_hash: v.prev_block_hash.into(),
        }
    }
}

impl From<electrs_client::BlockMeta> for crate::tokens::BlockHeader {
    fn from(v: electrs_client::BlockMeta) -> Self {
        Self {
            number: v.height,
            hash: v.block_hash.into(),
            prev_hash: v.prev_block_hash.into(),
        }
    }
}

impl From<&crate::tokens::BlockHeader> for electrs_client::BlockMeta {
    fn from(v: &crate::tokens::BlockHeader) -> Self {
        Self {
            height: v.number,
            block_hash: v.hash.into(),
            prev_block_hash: v.prev_hash.into(),
        }
    }
}

#[derive(
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Clone,
    bytemuck::Pod,
    bytemuck::Zeroable,
    Copy,
    Hash,
    Deserialize,
    Serialize,
)]
#[repr(C, packed)]
pub struct Outpoint {
    pub txid: [u8; 32],
    pub vout: u32,
}

impl std::fmt::Display for Outpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let txid = hex::encode(self.txid.iter().copied().rev().collect_vec());
        let vout = self.vout;
        write!(f, "{txid}i{vout}")
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[repr(u8)]
pub enum ParsedTokenAction {
    Deploy {
        tick: TokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
    },
    Mint {
        tick: TokenTick,
        amt: Fixed128,
    },
    DeployTransfer {
        tick: TokenTick,
        amt: Fixed128,
    },
    SpentTransfer {
        outpoint: Option<OutPoint>,
        tick: TokenTick,
        amt: Fixed128,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[repr(u8)]
pub enum ParsedTokenActionRest {
    Deploy {
        tick: TokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
    },
    Mint {
        tick: TokenTick,
        amt: Fixed128,
    },
    DeployTransfer {
        tick: TokenTick,
        amt: Fixed128,
    },
    SpentTransfer {
        outpoint: Option<OutPoint>,
        tick: TokenTick,
        amt: Fixed128,
    },
}

impl From<ParsedTokenAction> for ParsedTokenActionRest {
    fn from(value: ParsedTokenAction) -> Self {
        match value {
            ParsedTokenAction::Deploy {
                tick,
                max,
                lim,
                dec,
            } => ParsedTokenActionRest::Deploy {
                tick,
                max,
                lim,
                dec,
            },
            ParsedTokenAction::Mint { tick, amt } => ParsedTokenActionRest::Mint { tick, amt },
            ParsedTokenAction::DeployTransfer { tick, amt } => {
                ParsedTokenActionRest::DeployTransfer { tick, amt }
            }
            ParsedTokenAction::SpentTransfer {
                tick,
                amt,
                outpoint,
            } => ParsedTokenActionRest::SpentTransfer {
                tick,
                amt,
                outpoint,
            },
        }
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C, packed)]
pub struct HistoryLocation {
    pub offset: u64,
    pub outpoint: Outpoint,
}

impl std::fmt::Display for HistoryLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let outpoint = self.outpoint.to_string();
        let offest = self.offset;
        write!(f, "{outpoint}i{offest}")
    }
}

impl FromStr for Outpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('i').collect();
        if parts.len() != 2 {
            anyhow::bail!("Wrong len");
        }
        let txid = hex::decode(parts[0])
            .anyhow_with("Not hex")?
            .into_iter()
            .rev()
            .collect_vec();
        let txid: [u8; 32] = txid.try_into().map_err(|_| "Not array of 32").anyhow()?;
        let vout = parts[1].parse().map_err(|_| "Not u32").anyhow()?;

        Ok(Outpoint { txid, vout })
    }
}

impl<'de> serde::Deserialize<'de> for HistoryLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = <String>::deserialize(deserializer)?;
        let mut iter = v.split('i');

        let txid = iter.next();
        let vout = iter.next();
        let offset = iter.next();

        if iter.next().is_some() {
            return Err(serde::de::Error::custom("Many items"));
        }

        match (txid, vout, offset) {
            (Some(txid), Some(vout), Some(offset)) => {
                let offset: u64 = offset.parse().map_err(serde::de::Error::custom)?;
                let outpoint = Outpoint::from_str(&format!("{txid}i{vout}"))
                    .map_err(serde::de::Error::custom)?;

                Ok(Self { outpoint, offset })
            }
            _ => Err(serde::de::Error::custom("Wrong type")),
        }
    }
}

impl serde::Serialize for HistoryLocation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}
