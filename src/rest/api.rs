use nintypes::common::inscriptions::Outpoint;
use validator::Validate;

use super::*;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AddressTokenBalance {
    #[serde(serialize_with = "serialize_original_token_tick")]
    pub tick: OriginalTokenTick,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}

#[derive(Deserialize)]
pub struct AddressTokenHistoryArgs {
    pub offset: Option<u64>,
    pub limit: Option<usize>,
    pub tick: String,
}

#[derive(Deserialize)]
pub struct SubscribeArgs {
    #[serde(default)]
    pub addresses: Option<HashSet<String>>,
    #[serde(default)]
    pub tokens: Option<HashSet<String>>,
}

#[derive(Serialize)]
pub struct Status {
    pub height: u32,
    pub proof: String,
    pub blockhash: String,
}

#[derive(Serialize)]
pub struct ProofOfHistory {
    pub height: u32,
    pub hash: String,
}

#[derive(Deserialize)]
pub struct ProofHistoryArgs {
    pub offset: Option<u32>,
    pub limit: Option<usize>,
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

#[derive(Serialize)]
pub struct AddressTokenId {
    pub id: u64,
    pub address: String,
    #[serde(serialize_with = "serialize_original_token_tick")]
    pub tick: OriginalTokenTick,
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

#[derive(Serialize)]
pub struct History {
    #[serde(flatten)]
    pub address_token: AddressTokenId,
    pub height: u32,
    #[serde(flatten)]
    pub action: TokenAction,
}

impl History {
    pub async fn new(
        height: u32,
        action: TokenHistoryDB,
        address_token: crate::tokens::AddressTokenId,
        server: &Server,
    ) -> anyhow::Result<Self> {
        let keys = [action.address().copied(), Some(address_token.address)]
            .into_iter()
            .flatten();

        let addresses = server.load_addresses(keys, height).await?;

        Ok(Self {
            height,
            action: TokenAction::from_with_addresses(action, &addresses),
            address_token: AddressTokenId {
                address: addresses.get(&address_token.address).unwrap().clone(),
                id: address_token.id,
                tick: address_token.token,
            },
        })
    }
}

#[derive(Serialize)]
pub struct AddressHistory {
    #[serde(flatten)]
    pub history: History,
    pub created: u32,
}

impl AddressHistory {
    pub async fn new(
        height: u32,
        action: TokenHistoryDB,
        address_token: crate::tokens::AddressTokenId,
        server: &Server,
    ) -> anyhow::Result<Self> {
        let history = History::new(height, action, address_token, server).await?;
        let created = server.db.block_info.get(height).anyhow()?.created;
        Ok(Self { history, created })
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum TokenAction {
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
        recipient: String,
        txid: Txid,
        vout: u32,
    },
    Receive {
        amt: Fixed128,
        sender: String,
        txid: Txid,
        vout: u32,
    },
    SendReceive {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
}

impl From<server::HistoryValueEvent> for TokenAction {
    fn from(value: server::HistoryValueEvent) -> Self {
        match value.action {
            server::TokenHistoryEvent::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            } => Self::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            },
            server::TokenHistoryEvent::DeployTransfer { amt, txid, vout } => {
                Self::DeployTransfer { amt, txid, vout }
            }
            server::TokenHistoryEvent::Mint { amt, txid, vout } => Self::Mint { amt, txid, vout },
            server::TokenHistoryEvent::Send {
                amt,
                recipient,
                txid,
                vout,
            } => Self::Send {
                amt,
                recipient,
                txid,
                vout,
            },
            server::TokenHistoryEvent::Receive {
                amt,
                sender,
                txid,
                vout,
            } => Self::Receive {
                amt,
                sender,
                txid,
                vout,
            },
            server::TokenHistoryEvent::SendReceive { amt, txid, vout } => {
                Self::SendReceive { amt, txid, vout }
            }
        }
    }
}

impl TokenAction {
    pub fn from_with_addresses(
        value: TokenHistoryDB,
        addresses: &HashMap<FullHash, String>,
    ) -> Self {
        match value {
            TokenHistoryDB::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            } => TokenAction::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            },
            TokenHistoryDB::Mint { amt, txid, vout } => TokenAction::Mint { amt, txid, vout },
            TokenHistoryDB::DeployTransfer { amt, txid, vout } => {
                TokenAction::DeployTransfer { amt, txid, vout }
            }
            TokenHistoryDB::Send {
                amt,
                recipient,
                txid,
                vout,
            } => TokenAction::Send {
                amt,
                recipient: addresses.get(&recipient).unwrap().clone(),
                txid,
                vout,
            },
            TokenHistoryDB::Receive {
                amt,
                sender,
                txid,
                vout,
            } => TokenAction::Receive {
                amt,
                sender: addresses.get(&sender).unwrap().clone(),
                txid,
                vout,
            },
            TokenHistoryDB::SendReceive { amt, txid, vout } => {
                TokenAction::SendReceive { amt, txid, vout }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Default, Validate)]
pub struct HoldersArgs {
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = utils::page_size_default(), max = 20))]
    pub page_size: usize,
    #[validate(range(min = 1))]
    #[serde(default = "utils::first_page")]
    pub page: usize,
    #[validate(custom(function = "utils::validate_tick"))]
    pub tick: String,
}

#[derive(Serialize)]
pub struct Holder {
    pub rank: usize,
    pub address: String,
    pub balance: String,
    pub percent: String,
}

#[derive(Serialize, Default)]
pub struct Holders {
    pub pages: usize,
    pub count: usize,
    pub max_percent: Decimal,
    pub holders: Vec<Holder>,
}

#[derive(Serialize)]
pub struct Token {
    pub height: u32,
    pub created: u32,
    #[serde(serialize_with = "serialize_original_token_tick")]
    pub tick: OriginalTokenTick,
    pub genesis: InscriptionId,
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

#[derive(Deserialize, Default, Validate)]
pub struct TokenArgs {
    #[validate(custom(function = "utils::validate_tick"))]
    pub tick: String,
}

#[derive(Deserialize, Default)]
pub enum TokenSortBy {
    DeployTimeAsc,
    DeployTimeDesc,
    HoldersAsc,
    HoldersDesc,
    TransactionsAsc,
    #[default]
    TransactionsDesc,
}

#[derive(Deserialize, Default)]
pub enum TokenFilterBy {
    #[default]
    All,
    Completed,
    InProgress,
}

#[derive(Deserialize, Default, Validate)]
pub struct TokensArgs {
    #[serde(default = "utils::page_size_default")]
    #[validate(range(min = utils::page_size_default(), max = 20))]
    pub page_size: usize,
    #[validate(range(min = 1))]
    #[serde(default = "utils::first_page")]
    pub page: usize,
    #[serde(default)]
    pub sort_by: TokenSortBy,
    #[serde(default)]
    pub filter_by: TokenFilterBy,
    #[validate(length(min = 1, max = 4))]
    pub search: Option<String>,
}

#[derive(Serialize)]
pub struct TokensResult {
    pub pages: usize,
    pub count: usize,
    pub tokens: Vec<Token>,
}

#[derive(Deserialize)]
pub struct AddressTokenBalanceArgs {
    pub offset: Option<Outpoint>,
}

#[derive(Serialize, Deserialize)]
pub struct TokenBalance {
    #[serde(serialize_with = "serialize_original_token_tick")]
    pub tick: OriginalTokenTick,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}

#[derive(Serialize)]
pub struct TokenTransferProof {
    pub amt: Fixed128,
    #[serde(serialize_with = "serialize_original_token_tick")]
    pub tick: OriginalTokenTick,
    pub height: u32,
}

fn serialize_original_token_tick<S>(
    token: &OriginalTokenTick,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let str = token.to_string();
    serializer.serialize_str(&str)
}
