use std::ops::RangeInclusive;

use bellscoin::consensus;

use crate::tokens::proto::MintProtoWrapper;

use super::*;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressToken {
    pub address: FullHash,
    pub token: OriginalTokenTick,
}

impl AddressToken {
    pub fn search(address: FullHash) -> RangeInclusive<AddressToken> {
        let start = AddressToken {
            address,
            token: [0; 4].into(),
        };
        let end = AddressToken {
            address,
            token: [u8::MAX; 4].into(),
        };

        start..=end
    }
}

impl From<AddressTokenId> for AddressToken {
    fn from(value: AddressTokenId) -> Self {
        Self {
            address: value.address,
            token: value.token,
        }
    }
}

impl db::Pebble for AddressToken {
    type Inner = Self;

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Self {
            address: v[..32].try_into().anyhow()?,
            token: OriginalTokenTick(v[32..].try_into().expect("Expected [u8;4], but got more")),
        })
    }

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4);
        result.extend(v.address);
        result.extend(v.token.0);
        Cow::Owned(result)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressTokenId {
    pub address: FullHash,
    pub token: OriginalTokenTick,
    pub id: u64,
}

impl db::Pebble for AddressTokenId {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4 + 8);
        result.extend(v.address);
        result.extend(v.token.0);
        result.extend(v.id.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address: FullHash = v[..32].try_into().anyhow()?;
        let token = OriginalTokenTick(v[32..v.len() - 8].try_into().anyhow()?);
        let id = u64::from_be_bytes(v[v.len() - 8..].try_into().anyhow()?);

        Ok(Self { address, id, token })
    }
}

impl db::Pebble for Vec<AddressTokenId> {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::new();
        for item in v {
            result.extend(AddressTokenId::get_bytes(item).into_owned());
        }
        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        v.chunks(32 + 4 + 8)
            .map(|x| AddressTokenId::from_bytes(Cow::Borrowed(x)))
            .collect()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialOrd, Ord, PartialEq, Eq, Clone, Default)]
pub struct TokenBalance {
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers_count: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenHistoryDB {
    Deploy {
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
        txid: Txid,
        vout: u32,
    },
    Mint {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    DeployTransfer {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    Send {
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Receive {
        amt: Fixed128,
        sender: FullHash,
        txid: Txid,
        vout: u32,
    },
    SendReceive {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
}

#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct HistoryValue {
    pub height: u32,
    pub action: TokenHistoryDB,
}

impl TokenHistoryDB {
    pub fn from_token_history(token_history: HistoryTokenAction) -> Self {
        match token_history {
            HistoryTokenAction::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
                ..
            } => TokenHistoryDB::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            },
            HistoryTokenAction::Mint {
                amt, txid, vout, ..
            } => TokenHistoryDB::Mint { amt, txid, vout },
            HistoryTokenAction::DeployTransfer {
                amt, txid, vout, ..
            } => TokenHistoryDB::DeployTransfer { amt, txid, vout },
            HistoryTokenAction::Send {
                amt,
                recipient,
                sender,
                txid,
                vout,
                ..
            } => {
                if sender == recipient {
                    TokenHistoryDB::SendReceive { amt, txid, vout }
                } else {
                    TokenHistoryDB::Send {
                        amt,
                        recipient,
                        txid,
                        vout,
                    }
                }
            }
        }
    }

    pub fn address(&self) -> Option<&FullHash> {
        match self {
            TokenHistoryDB::Receive { sender, .. } => Some(sender),
            TokenHistoryDB::Send { recipient, .. } => Some(recipient),
            _ => None,
        }
    }

    pub fn outpoint(&self) -> OutPoint {
        match self {
            TokenHistoryDB::Deploy { txid, vout, .. }
            | TokenHistoryDB::Mint { txid, vout, .. }
            | TokenHistoryDB::DeployTransfer { txid, vout, .. }
            | TokenHistoryDB::Send { txid, vout, .. }
            | TokenHistoryDB::Receive { txid, vout, .. }
            | TokenHistoryDB::SendReceive { txid, vout, .. } => OutPoint {
                txid: *txid,
                vout: *vout,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenBalanceRest {
    pub tick: OriginalTokenTick,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}

#[derive(Serialize, Deserialize)]
pub struct TokenProtoRest {
    pub genesis: InscriptionId,
    pub tick: OriginalTokenTick,
    pub max: u64,
    pub lim: u64,
    pub dec: u8,
    pub supply: Fixed128,
    pub mint_count: u64,
    pub transfer_count: u64,
    pub holders: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
pub struct AddressLocation {
    pub address: FullHash,
    pub location: Location,
}

impl AddressLocation {
    pub fn search(address: FullHash, outpoint: Option<OutPoint>) -> RangeInclusive<Self> {
        if let Some(outpoint) = outpoint {
            return Self::search_with_outpoint(address, outpoint);
        }

        let start = Self {
            address,
            location: Location {
                outpoint: OutPoint {
                    txid: Txid::all_zeros(),
                    vout: 0,
                },
                offset: 0,
            },
        };
        let end = Self {
            address,
            location: Location {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([u8::MAX; 32]),
                    vout: u32::MAX,
                },
                offset: u64::MAX,
            },
        };

        start..=end
    }

    fn search_with_outpoint(address: FullHash, outpoint: OutPoint) -> RangeInclusive<Self> {
        let start = Self {
            address,
            location: Location {
                outpoint,
                offset: 0,
            },
        };
        let end = Self {
            address,
            location: Location {
                outpoint,
                offset: u64::MAX,
            },
        };

        start..=end
    }
}

impl db::Pebble for AddressLocation {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 44);

        result.extend(v.address);

        result.extend(consensus::serialize(&v.location.outpoint));
        result.extend(v.location.offset.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address = v[..32].try_into().anyhow()?;
        let outpoint: OutPoint = consensus::deserialize(&v[32..32 + 36])?;
        let offset = u64::from_be_bytes(v[32 + 32 + 4..].try_into().anyhow()?);

        Ok(Self {
            address,
            location: Location { outpoint, offset },
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4ActionErr {
    NotDeployed,
    AlreadyDeployed,
    ReachDecBound,
    ReachLimBound,
    SupplyMinted,
    InsufficientBalance,
    Transferred,
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
    Unknown(String),
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4Error {
    Action(Brc4ActionErr),
    Parse(Brc4ParseErr),
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash, Serialize, Deserialize)]
pub struct OriginalTokenTick(pub [u8; 4]);

impl TryFrom<Vec<u8>> for OriginalTokenTick {
    type Error = anyhow::Error;

    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(Self(
            v.try_into()
                .map_err(|_| anyhow::Error::msg("Invalid byte length"))?,
        ))
    }
}

impl From<[u8; 4]> for OriginalTokenTick {
    fn from(v: [u8; 4]) -> Self {
        Self(v)
    }
}
impl std::fmt::Debug for OriginalTokenTick {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}
impl Display for OriginalTokenTick {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.0))
    }
}
impl FromStr for OriginalTokenTick {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.as_bytes().try_into().anyhow_with("Invalid tick")?))
    }
}
impl From<OriginalTokenTick> for LowerCaseTokenTick {
    fn from(value: OriginalTokenTick) -> Self {
        LowerCaseTokenTick::from(value.0)
    }
}

impl From<&OriginalTokenTick> for LowerCaseTokenTick {
    fn from(value: &OriginalTokenTick) -> Self {
        LowerCaseTokenTick::from(&value.0)
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
        OutPoint {
            txid: val.txid,
            vout: val.index,
        }
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

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenAction {
    /// Deploy new token action.
    Deploy {
        genesis: InscriptionId,
        proto: DeployProtoDB,
        owner: FullHash,
    },
    /// Mint new token action.
    Mint {
        owner: FullHash,
        proto: MintProtoWrapper,
        txid: Txid,
        vout: u32,
    },
    /// Transfer token action.
    Transfer {
        location: Location,
        owner: FullHash,
        proto: MintProtoWrapper,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenTransfer {
    pub outpoint: OutPoint,
    pub amount: Fixed128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMeta {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenMetaDB {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

impl TokenMetaDB {
    pub fn is_completed(&self) -> bool {
        self.proto.is_completed()
    }
}

impl From<TokenMeta> for TokenMetaDB {
    fn from(meta: TokenMeta) -> Self {
        TokenMetaDB {
            genesis: meta.genesis,
            proto: meta.proto,
        }
    }
}

impl From<TokenMetaDB> for TokenMeta {
    fn from(meta: TokenMetaDB) -> Self {
        TokenMeta {
            genesis: meta.genesis,
            proto: meta.proto,
        }
    }
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
        Ok(Self(
            FromStr::from_str(&String::deserialize(deserializer)?)
                .map_err(serde::de::Error::custom)?,
        ))
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

#[derive(Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LowerCaseTokenTick(pub Vec<u8>);

impl<T: AsRef<[u8]>> From<T> for LowerCaseTokenTick {
    fn from(value: T) -> Self {
        LowerCaseTokenTick(
            String::from_utf8_lossy(value.as_ref())
                .to_lowercase()
                .as_bytes()
                .to_vec(),
        )
    }
}

impl std::ops::Deref for LowerCaseTokenTick {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for LowerCaseTokenTick {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl db::Pebble for LowerCaseTokenTick {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        Cow::Borrowed(&v.0)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Self(v.into_owned()))
    }
}
