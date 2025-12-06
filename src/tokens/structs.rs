use super::*;

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
pub struct AddressOutPoint {
    pub address: FullHash,
    pub outpoint: OutPoint,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4ParseErr {
    WrongContentType,
    WrongProtocol,
    DecimalEmpty,
    DecimalOverflow,
    DecimalPlusMinus,
    DecimalDotStartEnd,
    DecimalSpaces,
    InvalidDigit,
    InvalidUtf8,
    SelfMint5Byte,
    HeightTooLow5Byte,
    ZeroLimMax,
    ZeroAmt,
    Unknown(String),
}

/// Token tick in the original case (same as in the deploy)
#[derive(Clone, Hash, Eq, PartialEq)]
pub struct OriginalTokenTickRest(pub Vec<u8>);

impl schemars::JsonSchema for OriginalTokenTickRest {
    fn schema_name() -> Cow<'static, str> {
        "OriginalTokenTick".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^.+$"
        })
    }
}

impl Serialize for OriginalTokenTickRest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let str = String::from_utf8_lossy(&self.0);
        serializer.serialize_str(&str)
    }
}

impl<'de> Deserialize<'de> for OriginalTokenTickRest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let b = s.as_bytes();
        if !(b.len() == 4 || b.len() == 5) {
            return Err(serde::de::Error::custom("Invalid tick length"));
        }
        Ok(Self(b.to_vec()))
    }
}

impl Display for OriginalTokenTickRest {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.0))
    }
}

impl std::fmt::Debug for OriginalTokenTickRest {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl AsRef<[u8]> for OriginalTokenTickRest {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<OriginalTokenTick> for OriginalTokenTickRest {
    fn from(value: OriginalTokenTick) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OriginalTokenTick {
    Bytes4([u8; 4]),
    Bytes5([u8; 5]),
}

impl OriginalTokenTick {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            OriginalTokenTick::Bytes4(v) => v,
            OriginalTokenTick::Bytes5(v) => v,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            OriginalTokenTick::Bytes4(_) => 4,
            OriginalTokenTick::Bytes5(_) => 5,
        }
    }
}

impl Ord for OriginalTokenTick {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for OriginalTokenTick {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl TryFrom<Vec<u8>> for OriginalTokenTick {
    type Error = anyhow::Error;
    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        match v.len() {
            4 => Ok(OriginalTokenTick::Bytes4(v.try_into().map_err(|_| anyhow::Error::msg("Invalid length"))?)),
            5 => Ok(OriginalTokenTick::Bytes5(v.try_into().map_err(|_| anyhow::Error::msg("Invalid length"))?)),
            _ => anyhow::bail!("Invalid tick length"),
        }
    }
}

impl From<OriginalTokenTickRest> for OriginalTokenTick {
    fn from(value: OriginalTokenTickRest) -> Self {
        OriginalTokenTick::try_from(value.0).expect("Invalid tick length")
    }
}

impl From<[u8; 4]> for OriginalTokenTick {
    fn from(v: [u8; 4]) -> Self {
        OriginalTokenTick::Bytes4(v)
    }
}

impl std::fmt::Debug for OriginalTokenTick {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}
impl Display for OriginalTokenTick {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(self.as_bytes()))
    }
}
impl FromStr for OriginalTokenTick {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        OriginalTokenTick::try_from(s.as_bytes().to_vec())
    }
}

impl From<OriginalTokenTick> for LowerCaseTokenTick {
    fn from(value: OriginalTokenTick) -> Self {
        LowerCaseTokenTick::from(value.as_bytes())
    }
}

impl From<&OriginalTokenTick> for LowerCaseTokenTick {
    fn from(value: &OriginalTokenTick) -> Self {
        LowerCaseTokenTick::from(value.as_bytes())
    }
}

impl Default for OriginalTokenTick {
    fn default() -> Self {
        OriginalTokenTick::Bytes4([0u8; 4])
    }
}

#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq)]
pub struct InscriptionId {
    pub txid: Txid,
    pub index: u32,
}

impl<'de> Deserialize<'de> for InscriptionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(DeserializeFromStr::deserialize(deserializer)?.0)
    }
}

impl Serialize for InscriptionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl Display for InscriptionId {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}i{}", self.txid, self.index)
    }
}

impl From<InscriptionId> for OutPoint {
    fn from(val: InscriptionId) -> Self {
        OutPoint::new(val.txid, val.index)
    }
}

impl From<OutPoint> for InscriptionId {
    fn from(outpoint: OutPoint) -> Self {
        Self {
            txid: outpoint.txid,
            index: outpoint.vout,
        }
    }
}

impl FromStr for InscriptionId {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(char) = s.chars().find(|char| !char.is_ascii()) {
            return Err(ParseError::Character(char));
        }

        const TXID_LEN: usize = 64;
        const MIN_LEN: usize = TXID_LEN + 2;

        if s.len() < MIN_LEN {
            return Err(ParseError::Length(s.len()));
        }

        let txid = &s[..TXID_LEN];

        let separator = s.chars().nth(TXID_LEN).ok_or(ParseError::Separator(' '))?;

        if separator != 'i' {
            return Err(ParseError::Separator(separator));
        }

        let vout = &s[TXID_LEN + 1..];

        Ok(Self {
            txid: txid.parse().map_err(ParseError::Txid)?,
            index: vout.parse().map_err(ParseError::Index)?,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenAction {
    /// Deploy new token action.
    Deploy { genesis: InscriptionId, proto: DeployProtoDB, owner: FullHash },
    /// Mint new token action.
    Mint { owner: FullHash, proto: MintProto, txid: Txid, vout: u32 },
    /// Transfer token action.
    Transfer {
        location: Location,
        owner: FullHash,
        proto: TransferProto,
        txid: Txid,
        vout: u32,
    },
    /// Founded move of transfer action.
    Transferred {
        // TokenAction::Transfer location
        transfer_location: Location,
        // if leaked then sender = recipient
        // if burnt them recipient = OP_RETURN_HASH
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
}

/// Token transfer
#[derive(Serialize, Deserialize, Debug, Clone, schemars::JsonSchema)]
pub struct TokenTransfer {
    pub outpoint: crate::rest::OutPoint,
    pub amount: Fixed128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMeta {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

#[derive(Clone, Debug)]
pub struct InscriptionTemplate {
    pub genesis: InscriptionId,
    pub location: Location,
    pub content_type: Option<String>,
    pub owner: FullHash,
    pub value: u64,
    pub content: Option<Vec<u8>>,
    pub leaked: bool,
    // ord/OPI compatibility fields (used for p2tr coins)
    pub input_index: u32,
    pub envelope_offset: u32,
    pub duplicate_field: bool,
    pub incomplete_field: bool,
    pub unrecognized_even_field: bool,
    pub has_pointer: bool,
    pub pushnum: bool,
    pub stutter: bool,
    pub cursed_for_brc20: bool,
    pub unbound: bool,
    pub reinscription: bool,
    pub vindicated: bool,
    pub pointer_value: Option<u64>,
}

pub(crate) struct DeserializeFromStr<T: FromStr>(pub(crate) T);

impl<'de, T: FromStr> Deserialize<'de> for DeserializeFromStr<T>
where
    T::Err: Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(FromStr::from_str(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)?))
    }
}

#[derive(Debug)]
pub enum ParseError {
    Character(char),
    Length(usize),
    Separator(char),
    Txid(bellscoin::hashes::hex::Error),
    Index(std::num::ParseIntError),
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::Character(c) => write!(f, "invalid character: '{c}'"),
            Self::Length(len) => write!(f, "invalid length: {len}"),
            Self::Separator(c) => write!(f, "invalid separator: `{c}`"),
            Self::Txid(err) => write!(f, "invalid txid: {err}"),
            Self::Index(err) => write!(f, "invalid index: {err}"),
        }
    }
}

impl std::error::Error for ParseError {}
