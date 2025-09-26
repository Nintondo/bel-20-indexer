use nintypes::common::inscriptions::Outpoint;

use super::*;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AddressTokenBalance {
    pub tick: OriginalTokenTickRest,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}

#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct TokenEventsArgs {
    /// Offset by event id
    pub offset: Option<u64>,
    /// Limit of the number of events to return.
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 100))]
    pub limit: usize,
    /// Search by txid or outpoint
    pub search: Option<String>,
}

/// Address token history query arguments
#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct AddressTokenHistoryArgs {
    /// Event ID of the last item from the previous page.
    pub offset: Option<u64>,
    /// Limit of the number of events to return.
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 100))]
    pub limit: usize,
    pub tick: OriginalTokenTickRest,
}

#[derive(Deserialize)]
pub struct SubscribeArgs {
    #[serde(default)]
    pub addresses: Option<HashSet<String>>,
    #[serde(default)]
    pub tokens: Option<HashSet<OriginalTokenTickRest>>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct Status {
    /// Current height of the blockchain
    pub height: u32,
    /// Proof of history of the last block
    pub proof: String,
    /// Hash of the last block
    pub blockhash: String,
    /// Version of the indexer
    pub version: String,
    /// Uptime of the indexer in seconds
    pub uptime_secs: u64,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ProofOfHistory {
    /// Height of the block
    pub height: u32,
    /// Proof of history of the block
    pub hash: String,
}

#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct ProofHistoryArgs {
    /// Offset by block height
    pub offset: Option<u32>,
    /// Limit of the number of blocks to return.
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 100))]
    pub limit: usize,
}

#[derive(Serialize)]
pub struct Reorg {
    pub event_type: String,
    pub blocks_count: u32,
    pub new_height: u32,
}

#[derive(Serialize)]
pub struct NewBlock {
    pub event_type: String,
    pub height: u32,
    pub proof: sha256::Hash,
    pub blockhash: BlockHash,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct AddressTokenId {
    /// Unique ID of the token event
    pub id: u64,
    /// Address of the token event actor
    pub address: String,
    pub tick: OriginalTokenTickRest,
}

impl From<server::AddressTokenIdEvent> for AddressTokenId {
    fn from(value: server::AddressTokenIdEvent) -> Self {
        Self {
            address: value.address,
            id: value.id,
            tick: value.token,
        }
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct History {
    #[serde(flatten)]
    pub address_token: AddressTokenId,
    /// Block height of the block in which the history was created
    pub height: u32,
    #[serde(flatten)]
    pub action: TokenAction,
}

impl History {
    pub fn new(height: u32, action: TokenHistoryDB, address_token: AddressTokenIdDB, server: &Server) -> anyhow::Result<Self> {
        let keys = [action.address().copied(), Some(address_token.address)].into_iter().flatten();

        let addresses = server.load_addresses(keys)?;

        Ok(Self {
            height,
            action: TokenAction::from_with_addresses(action, &addresses),
            address_token: AddressTokenId {
                address: addresses.get(&address_token.address),
                id: address_token.id,
                tick: address_token.token.into(),
            },
        })
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct AddressHistory {
    #[serde(flatten)]
    pub history: History,
    /// Block timestamp of the block in which the history was created (in seconds since UNIX epoch)
    pub created: u32,
}

impl AddressHistory {
    pub fn new(height: u32, action: TokenHistoryDB, address_token: AddressTokenIdDB, server: &Server) -> anyhow::Result<Self> {
        let history = History::new(height, action, address_token, server)?;
        let created = server.db.block_info.get(height).anyhow()?.created;
        Ok(Self { history, created })
    }
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(tag = "type")]
pub enum TokenAction {
    /// Deploy event
    Deploy { max: Fixed128, lim: Fixed128, dec: u8, txid: Txid, vout: u32 },
    /// Mint event
    Mint { amt: Fixed128, txid: Txid, vout: u32 },
    /// Deploy transfer event
    DeployTransfer { amt: Fixed128, txid: Txid, vout: u32 },
    /// Send event
    Send { amt: Fixed128, recipient: String, txid: Txid, vout: u32 },
    /// Receive event
    Receive { amt: Fixed128, sender: String, txid: Txid, vout: u32 },
    /// SendReceive event
    SendReceive { amt: Fixed128, txid: Txid, vout: u32 },
}

impl From<server::HistoryValueEvent> for TokenAction {
    fn from(value: server::HistoryValueEvent) -> Self {
        match value.action {
            server::TokenHistoryEvent::Deploy { max, lim, dec, txid, vout } => Self::Deploy {
                max,
                lim,
                dec,
                txid: txid.into(),
                vout,
            },
            server::TokenHistoryEvent::DeployTransfer { amt, txid, vout } => Self::DeployTransfer { amt, txid: txid.into(), vout },
            server::TokenHistoryEvent::Mint { amt, txid, vout } => Self::Mint { amt, txid: txid.into(), vout },
            server::TokenHistoryEvent::Send { amt, recipient, txid, vout } => Self::Send {
                amt,
                recipient,
                txid: txid.into(),
                vout,
            },
            server::TokenHistoryEvent::Receive { amt, sender, txid, vout } => Self::Receive {
                amt,
                sender,
                txid: txid.into(),
                vout,
            },
            server::TokenHistoryEvent::SendReceive { amt, txid, vout } => Self::SendReceive { amt, txid: txid.into(), vout },
        }
    }
}

impl TokenAction {
    pub fn from_with_addresses(value: TokenHistoryDB, addresses: &AddressesFullHash) -> Self {
        match value {
            TokenHistoryDB::Deploy { max, lim, dec, txid, vout } => TokenAction::Deploy {
                max,
                lim,
                dec,
                txid: txid.into(),
                vout,
            },
            TokenHistoryDB::Mint { amt, txid, vout } => TokenAction::Mint { amt, txid: txid.into(), vout },
            TokenHistoryDB::DeployTransfer { amt, txid, vout } => TokenAction::DeployTransfer { amt, txid: txid.into(), vout },
            TokenHistoryDB::Send { amt, recipient, txid, vout } => TokenAction::Send {
                amt,
                recipient: addresses.get(&recipient),
                txid: txid.into(),
                vout,
            },
            TokenHistoryDB::Receive { amt, sender, txid, vout } => TokenAction::Receive {
                amt,
                sender: addresses.get(&sender),
                txid: txid.into(),
                vout,
            },
            TokenHistoryDB::SendReceive { amt, txid, vout } => TokenAction::SendReceive { amt, txid: txid.into(), vout },
        }
    }
}

#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct HoldersArgs {
    /// Page size of the holders
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 20))]
    pub page_size: usize,
    /// Page of the holders
    #[validate(range(min = 1))]
    #[serde(default = "utils::first_page")]
    pub page: usize,
    pub tick: OriginalTokenTickRest,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct HoldersStatsArgs {
    pub tick: OriginalTokenTickRest,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct Holder {
    /// Rank of the holder
    pub rank: usize,
    /// Address of the holder
    pub address: String,
    /// Balance of the holder
    pub balance: String,
    /// Percent of the total supply
    pub percent: String,
}

#[derive(Serialize, Default, schemars::JsonSchema)]
pub struct Holders {
    /// Number of pages
    pub pages: usize,
    /// Total number of holders
    pub count: usize,
    /// Max percent of the total supply
    pub max_percent: String,
    /// List of holders
    pub holders: Vec<Holder>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct Token {
    pub height: u32,
    pub created: u32,
    pub tick: OriginalTokenTickRest,
    pub genesis: RestInscriptionId,
    pub deployer: String,

    pub transactions: u32,
    pub holders: u32,
    pub supply: Fixed128,
    pub mint_percent: String,
    pub completed: bool,

    pub max: Fixed128,
    pub lim: Fixed128,
    pub dec: u8,
}

#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct TokenArgs {
    pub tick: OriginalTokenTickRest,
}

#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq, schemars::JsonSchema)]
pub struct RestInscriptionId {
    pub txid: rest::Txid,
    pub index: u32,
}

impl From<InscriptionId> for RestInscriptionId {
    fn from(value: InscriptionId) -> Self {
        Self {
            txid: value.txid.into(),
            index: value.index,
        }
    }
}

impl<'de> Deserialize<'de> for RestInscriptionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(DeserializeFromStr::deserialize(deserializer)?.0)
    }
}

impl Serialize for RestInscriptionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl Display for RestInscriptionId {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}i{}", self.txid, self.index)
    }
}

impl FromStr for RestInscriptionId {
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

#[derive(Deserialize, Default, schemars::JsonSchema)]
pub enum TokenSortBy {
    /// Sort by deploy time
    DeployTimeAsc,
    /// Sort by deploy time (descending)
    DeployTimeDesc,
    /// Sort by holders
    HoldersAsc,
    /// Sort by holders (descending)
    HoldersDesc,
    /// Sort by transactions
    TransactionsAsc,
    /// Sort by transactions (descending)
    #[default]
    TransactionsDesc,
}

#[derive(Deserialize, Default, schemars::JsonSchema)]
pub enum TokenFilterBy {
    /// All tokens
    #[default]
    All,
    /// Tokens with completed minting
    Completed,
    /// Tokens with in progress minting
    InProgress,
}

#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct TokensArgs {
    /// Page size of the tokens
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 100))]
    pub page_size: usize,
    /// Page of the tokens
    #[validate(range(min = 1))]
    #[serde(default = "utils::first_page")]
    pub page: usize,
    #[serde(default)]
    /// Sorting of the tokens
    pub sort_by: TokenSortBy,
    /// Filtering of the tokens
    #[serde(default)]
    pub filter_by: TokenFilterBy,
    /// Search by token tick
    pub search: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct TokensResult {
    pub pages: usize,
    pub count: usize,
    pub tokens: Vec<Token>,
}

/// Address token balance query params
#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct AddressTokenBalanceArgs {
    /// Outpoint of the last item from the previous page.
    pub offset: Option<Outpoint>,
    /// Limit of the number of tokens to return.
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 300))]
    pub limit: usize,
}

/// Address tokens query arguments
#[derive(Deserialize, Validate, schemars::JsonSchema)]
pub struct AddressTokensArgs {
    /// Token tick of the last item from the previous page.
    pub offset: Option<OriginalTokenTickRest>,
    /// Limit of the number of tokens to return.
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = 1, max = 100))]
    pub limit: usize,
    /// Search query by token tick
    pub search: Option<String>,
}

/// Address token balance response
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct TokenBalance {
    pub tick: OriginalTokenTickRest,
    /// Balance of the token
    pub balance: Fixed128,
    /// Balance of the token that can be transferred
    pub transferable_balance: Fixed128,
    /// Number of transfers
    pub transfers_count: u64,
    /// List of transfers
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transfers: Vec<TokenTransfer>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct TokenTransferProof {
    /// Amount of the transfer
    pub amt: Fixed128,
    pub tick: OriginalTokenTickRest,
    /// Block height of the block in which the transfer was created
    pub height: u32,
}

#[derive(Deserialize)]
pub struct AllTickersQuery {
    #[serde(default)]
    pub block_height: Option<u32>,
}
