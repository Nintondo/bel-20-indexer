use crate::inscriptions::load_decoder;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
pub mod threads;
use crate::address_encoder::Decoder;
use core_utils::utils::client::AsyncClient;
use application::DEFAULT_HASH;
use application::{PASS, URL, USER};
use bellscoin::hashes::{Hash, sha256};
use bellscoin::{PublicKey, ScriptBuf};
use core_utils::db::tables::DB;
use core_utils::interfaces::server::{
    AddressesLoader, ClientPort, DBPort, EventSenderPort, HistoryHashGenerator, HoldersPort, TokenPort
};
use core_utils::types::full_hash::{ComputeScriptHash, FullHash};
use core_utils::types::holders::Holders;
use core_utils::types::server::{RawServerEvent, ServerEvent};
use core_utils::types::structs::{AddressTokenId, HistoryValue};
use core_utils::{IsOpReturnHash, NON_STANDARD_ADDRESS, OP_RETURN_ADDRESS};
use dutils::wait_token::WaitToken;

pub struct Server {
    pub db: Arc<DB>,
    pub event_sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub raw_event_sender: kanal::Sender<RawServerEvent>,
    pub token: WaitToken,
    pub client: Arc<AsyncClient>,
    pub holders: Arc<Holders>,
    pub address_decoder: Box<dyn Decoder>,
}

impl Server {
    pub async fn new(
        db_path: &str,
    ) -> anyhow::Result<(
        kanal::Receiver<RawServerEvent>,
        tokio::sync::broadcast::Sender<ServerEvent>,
        Self,
    )> {
        let (raw_tx, raw_rx) = kanal::unbounded();
        let (tx, _) = tokio::sync::broadcast::channel(30_000);
        let token = WaitToken::default();
        let db = Arc::new(DB::open(db_path));

        let server = Self {
            client: Arc::new(
                AsyncClient::new(
                    &URL,
                    Some(USER.to_string()),
                    Some(PASS.to_string()),
                    token.clone(),
                )
                .await?,
            ),
            address_decoder: load_decoder(),
            holders: Arc::new(Holders::init(&db)),
            db,
            raw_event_sender: raw_tx.clone(),
            token,
            event_sender: tx.clone(),
        };

        Ok((raw_rx, tx, server))
    }

    pub fn to_scripthash(&self, script_type: &str, script_str: &str) -> anyhow::Result<FullHash> {
        let Ok(pubkey) = PublicKey::from_str(script_str) else {
            return match script_type {
                "address" => self.address_to_scripthash(script_str),
                "scripthash" => Self::parse_scripthash(script_str),
                _ => anyhow::bail!("Invalid script type"),
            };
        };
        Ok(ScriptBuf::new_p2pk(&pubkey).compute_script_hash())
    }

    pub fn address_to_scripthash(&self, address: &str) -> anyhow::Result<FullHash> {
        self.address_decoder
            .decode(address)
            .map(|x| x.script_pubkey().compute_script_hash())
            .map_err(|_| anyhow::anyhow!(""))
    }

    fn parse_scripthash(scripthash: &str) -> anyhow::Result<FullHash> {
        let bytes = hex::decode(scripthash)?;
        bytes.try_into()
    }
}

impl DBPort for Server {
    fn get_db(&self) -> Arc<DB> {
        self.db.clone()
    }
}

impl HoldersPort for Server {
    fn get_holders(&self) -> Arc<Holders> {
        self.holders.clone()
    }
}

impl EventSenderPort for Server {
    fn get_event_sender(&self) -> tokio::sync::broadcast::Sender<ServerEvent> {
        self.event_sender.clone()
    }
    fn get_raw_event_sender(&self) -> kanal::Sender<RawServerEvent> {
        self.raw_event_sender.clone()
    }
}

impl TokenPort for Server {
    fn get_token(&self) -> WaitToken {
        self.token.clone()
    }
}

impl ClientPort<Arc<AsyncClient>> for Server {
    fn get_client(&self) -> Arc<AsyncClient> {
        self.client.clone()
    }
}

impl AddressesLoader for Server {
    async fn load_addresses(
        &self,
        keys: impl IntoIterator<Item = FullHash> + Send + Sync,
    ) -> anyhow::Result<HashMap<FullHash, String>> {
        let keys = keys.into_iter().collect::<HashSet<_>>();

        Ok(self
            .db
            .fullhash_to_address
            .multi_get(keys.iter())
            .into_iter()
            .zip(keys)
            .map(|(v, k)| {
                if k.is_op_return_hash() {
                    (k, OP_RETURN_ADDRESS.to_string())
                } else {
                    (k, v.unwrap_or(NON_STANDARD_ADDRESS.to_string()))
                }
            })
            .collect())
    }
}

impl HistoryHashGenerator for Server {
    fn generate_history_hash(
        prev_history_hash: sha256::Hash,
        history: &[(AddressTokenId, HistoryValue)],
        addresses: &HashMap<FullHash, String>,
    ) -> anyhow::Result<sha256::Hash> {
        let current_hash = if history.is_empty() {
            *DEFAULT_HASH
        } else {
            let mut buffer = Vec::<u8>::new();

            for (address_token, action) in history {
                let rest = core_utils::types::rest::rest_api::History {
                    height: action.height,
                    action: core_utils::types::rest::rest_api::TokenAction::from_with_addresses(
                        action.action.clone(),
                        addresses,
                    ),
                    address_token: core_utils::types::rest::rest_api::AddressTokenId {
                        address: addresses.get(&address_token.address).unwrap().clone(),
                        id: address_token.id,
                        tick: address_token.token,
                    },
                };
                let bytes = serde_json::to_vec(&rest)?;
                buffer.extend(bytes);
            }

            sha256::Hash::hash(&buffer)
        };

        let new_hash = {
            let mut buffer = prev_history_hash.as_byte_array().to_vec();
            buffer.extend_from_slice(current_hash.as_byte_array());
            sha256::Hash::hash(&buffer)
        };

        Ok(new_hash)
    }
}