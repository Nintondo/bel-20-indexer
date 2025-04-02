use crate::Fixed128;

use super::*;

use num_traits::FromPrimitive;
use serde::de::Error;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Protocol(pub Brc4Value, pub Option<Brc4ActionErr>);

fn bel_20_validate<'de, D>(val: &str) -> Result<Fixed128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    if val.starts_with('+') | val.starts_with('-') {
        return Err(Error::custom("value cannot start from + or -"));
    }
    if val.starts_with('.') | val.ends_with('.') {
        return Err(Error::custom("value cannot start or end with ."));
    }
    if val.starts_with(' ') | val.ends_with(' ') {
        return Err(Error::custom("value cannot contain spaces"));
    }
    match Fixed128::from_str(val) {
        Ok(v) => {
            if v > Fixed128::from(u64::MAX) {
                Err(Error::custom("value is too large"))
            } else {
                Ok(v)
            }
        }
        Err(e) => Err(Error::custom(e)),
    }
}

pub fn bel_20_decimal<'de, D>(deserializer: D) -> Result<Fixed128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = <&str as serde::Deserialize>::deserialize(deserializer)?;
    bel_20_validate::<D>(val)
}

pub fn bel_20_option_decimal<'de, D>(deserializer: D) -> Result<Option<Fixed128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = <Option<&str> as serde::Deserialize>::deserialize(deserializer)?;
    val.map(|x| bel_20_validate::<D>(x)).transpose()
}

pub fn bel_20_tick<'de, D>(deserializer: D) -> Result<TokenTick, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = <Cow<str> as serde::Deserialize>::deserialize(deserializer)?;
    let val = val.as_bytes().to_vec();

    if val.len() != 4 {
        return Err(Error::custom("invalid token tick"));
    }

    Ok(val.try_into().unwrap())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op")]
#[serde(rename_all = "lowercase")]
pub enum Brc4 {
    Mint {
        #[serde(flatten)]
        proto: MintProto,
    },
    Deploy {
        #[serde(flatten)]
        proto: DeployProto,
    },
    Transfer {
        #[serde(flatten)]
        proto: TransferProto,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "p")]
#[serde_as]
pub enum MintProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(deserialize_with = "bel_20_decimal")]
        amt: Fixed128,
    },
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "p")]
#[serde_as]
pub enum DeployProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(deserialize_with = "bel_20_decimal")]
        max: Fixed128,
        #[serde(default, deserialize_with = "bel_20_option_decimal")]
        lim: Option<Fixed128>,
        #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
        #[serde(default = "DeployProto::default_dec")]
        dec: u8,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeployProtoDB {
    pub tick: TokenTick,
    pub max: Fixed128,
    pub lim: Fixed128,
    pub dec: u8,
    pub supply: Fixed128,
    pub transfer_count: u64,
    pub mint_count: u64,
    pub height: u32,
    pub created: u32,
    pub deployer: FullHash,
    pub transactions: u32,
}

impl DeployProtoDB {
    pub fn is_completed(&self) -> bool {
        self.supply == Fixed128::from(self.max)
    }
    pub fn mint_percent(&self) -> Fixed128 {
        (rust_decimal::Decimal::from_u64(100).unwrap() * self.supply.into_decimal()
            / self.max.into_decimal())
        .into()
    }
}

impl DeployProto {
    pub const DEFAULT_DEC: u8 = 18;
    pub const MAX_DEC: u8 = 18;
    pub fn default_dec() -> u8 {
        Self::DEFAULT_DEC
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "p")]
#[serde_as]
pub enum TransferProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(deserialize_with = "bel_20_decimal")]
        amt: Fixed128,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TransferProtoDB {
    pub tick: TokenTick,
    pub amt: Fixed128,
    pub height: u32,
}

impl From<TransferProtoDB> for TransferProto {
    fn from(v: TransferProtoDB) -> Self {
        TransferProto::Bel20 {
            tick: v.tick,
            amt: v.amt,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Brc4Value {
    Mint {
        tick: TokenTick,
        amt: Fixed128,
    },
    Transfer {
        tick: TokenTick,
        amt: Fixed128,
    },
    Deploy {
        tick: TokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
    },
}

impl From<&DeployProto> for Brc4Value {
    fn from(v: &DeployProto) -> Self {
        match v {
            DeployProto::Bel20 {
                tick,
                max,
                lim,
                dec,
                ..
            } => Brc4Value::Deploy {
                tick: *tick,
                max: *max,
                lim: (*lim).unwrap_or(*max),
                dec: *dec,
            },
        }
    }
}

impl From<&MintProto> for Brc4Value {
    fn from(v: &MintProto) -> Self {
        match v {
            MintProto::Bel20 { tick, amt } => Brc4Value::Mint {
                tick: *tick,
                amt: *amt,
            },
        }
    }
}

impl From<&TransferProto> for Brc4Value {
    fn from(v: &TransferProto) -> Self {
        match v {
            TransferProto::Bel20 { tick, amt } => Brc4Value::Transfer {
                tick: *tick,
                amt: *amt,
            },
        }
    }
}
