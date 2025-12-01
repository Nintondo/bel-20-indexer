use super::*;

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

pub fn bel_20_tick<'de, D>(deserializer: D) -> Result<OriginalTokenTick, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = <Cow<str> as serde::Deserialize>::deserialize(deserializer)?;
    let val = val.as_bytes().to_vec();

    match OriginalTokenTick::try_from(val) {
        Ok(v) => Ok(v),
        Err(_) => Err(Error::custom("invalid token tick")),
    }
}

fn bool_from_string_default_false<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::<String>::deserialize(deserializer)?;
    Ok(match s.as_deref() {
        Some("true") => true,
        Some("false") => false,
        None => false,
        _ => return Err(Error::custom("self_mint must be a string 'true' or 'false'")),
    })
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
pub struct MintProto {
    #[serde(deserialize_with = "bel_20_tick")]
    pub tick: OriginalTokenTick,
    #[serde(deserialize_with = "bel_20_decimal")]
    pub amt: Fixed128,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct DeployProtoWrapper {
    #[serde(deserialize_with = "bel_20_tick")]
    pub tick: OriginalTokenTick,
    #[serde(deserialize_with = "bel_20_decimal")]
    pub max: Fixed128,
    #[serde(default, deserialize_with = "bel_20_option_decimal")]
    pub lim: Option<Fixed128>,
    #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
    #[serde(default = "DeployProto::default_dec")]
    pub dec: u8,
    #[serde(default, deserialize_with = "bool_from_string_default_false")]
    pub self_mint: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeployProto {
    #[serde(deserialize_with = "bel_20_tick")]
    pub tick: OriginalTokenTick,
    #[serde(deserialize_with = "bel_20_decimal")]
    pub max: Fixed128,
    #[serde(default, deserialize_with = "bel_20_option_decimal")]
    pub lim: Option<Fixed128>,
    #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
    #[serde(default = "DeployProto::default_dec")]
    pub dec: u8,
    #[serde(default, deserialize_with = "bool_from_string_default_false")]
    pub self_mint: bool,
}

impl DeployProto {
    pub const DEFAULT_DEC: u8 = 18;
    pub const MAX_DEC: u8 = 18;
    pub fn default_dec() -> u8 {
        Self::DEFAULT_DEC
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct TransferProto {
    #[serde(deserialize_with = "bel_20_tick")]
    pub tick: OriginalTokenTick,
    #[serde(deserialize_with = "bel_20_decimal")]
    pub amt: Fixed128,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Brc4Value {
    Mint { tick: OriginalTokenTick, amt: Fixed128 },
    Transfer { tick: OriginalTokenTick, amt: Fixed128 },
    Deploy { tick: OriginalTokenTick, max: Fixed128, lim: Fixed128, dec: u8 },
}

impl From<&DeployProto> for Brc4Value {
    fn from(v: &DeployProto) -> Self {
        Self::Deploy {
            tick: v.tick,
            max: v.max,
            lim: v.lim.unwrap_or(v.max),
            dec: v.dec,
        }
    }
}

impl From<&MintProto> for Brc4Value {
    fn from(v: &MintProto) -> Self {
        Self::Mint { tick: v.tick, amt: v.amt }
    }
}

impl From<&TransferProto> for Brc4Value {
    fn from(v: &TransferProto) -> Self {
        Self::Transfer { tick: v.tick, amt: v.amt }
    }
}
