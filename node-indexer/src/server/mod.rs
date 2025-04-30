use crate::DEFAULT_HASH;
use crate::client::AsyncClient;
use application::{PASS, URL, USER};
use bellscoin::BlockHash;
use bellscoin::hashes::{Hash, sha256};
use core_utils::db::tables::DB;
use core_utils::interfaces::server::AddressesLoader;
use core_utils::interfaces::server::{
    ClientPort, DBPort, EventSenderPort, HistoryHashGenerator, HoldersPort, LastIndexedAddressPort,
    TokenPort,
};
use core_utils::types::full_hash::FullHash;
use core_utils::types::holders::Holders;
use core_utils::types::rest::rest_api::{self, History};
use core_utils::types::server::{RawServerEvent, ServerEvent};
use core_utils::types::structs::{AddressTokenId, HistoryValue};
use core_utils::{IsOpReturnHash, NON_STANDARD_ADDRESS, OP_RETURN_ADDRESS};
use dutils::wait_token::WaitToken;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use threads::address_hash_saver::AddressesToLoad;

pub mod threads;

#[derive(Clone)]
pub struct Server {
    pub db: Arc<DB>,
    pub event_sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub raw_event_sender: kanal::Sender<RawServerEvent>,
    pub token: WaitToken,
    pub last_indexed_address_height: Arc<tokio::sync::RwLock<u32>>,
    pub addr_tx: Arc<kanal::Sender<AddressesToLoad>>,
    pub client: Arc<AsyncClient>,
    pub holders: Arc<Holders>,
}

impl Server {
    pub async fn new(
        db_path: &str,
    ) -> anyhow::Result<(
        kanal::Receiver<AddressesToLoad>,
        kanal::Receiver<RawServerEvent>,
        tokio::sync::broadcast::Sender<ServerEvent>,
        Self,
    )> {
        let (raw_tx, raw_rx) = kanal::unbounded();
        let (tx, _) = tokio::sync::broadcast::channel(30_000);
        let (addr_tx, addr_rx) = kanal::unbounded();
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
            addr_tx: Arc::new(addr_tx),
            holders: Arc::new(Holders::init(&db)),
            db,
            raw_event_sender: raw_tx.clone(),
            token,
            last_indexed_address_height: Arc::new(tokio::sync::RwLock::new(0)),
            event_sender: tx.clone(),
        };

        Ok((addr_rx, raw_rx, tx, server))
    }

    pub async fn new_hash(
        &self,
        height: u32,
        blockhash: BlockHash,
        history: &[(AddressTokenId, HistoryValue)],
    ) -> anyhow::Result<()> {
        let current_hash = if history.is_empty() {
            *DEFAULT_HASH
        } else {
            let mut res = Vec::<u8>::new();

            for (k, v) in history {
                let bytes = serde_json::to_vec(
                    &History::new(v.height, v.action.clone(), k.clone(), self).await?,
                )?;
                res.extend(bytes);
            }

            sha256::Hash::hash(&res)
        };

        let new_hash = {
            let prev_hash = self
                .db
                .proof_of_history
                .get(height - 1)
                .unwrap_or(*DEFAULT_HASH);
            let mut result = vec![];
            result.extend_from_slice(prev_hash.as_byte_array());
            result.extend_from_slice(current_hash.as_byte_array());

            sha256::Hash::hash(&result)
        };

        self.event_sender
            .send(ServerEvent::NewBlock(height, new_hash, blockhash))
            .ok();

        self.db.proof_of_history.set(height, new_hash);

        Ok(())
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
                let rest = rest_api::History {
                    height: action.height,
                    action: rest_api::TokenAction::from_with_addresses(
                        action.action.clone(),
                        addresses,
                    ),
                    address_token: rest_api::AddressTokenId {
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

impl LastIndexedAddressPort for Server {
    fn get_last_indexed_address_height(&self) -> Arc<tokio::sync::RwLock<u32>> {
        self.last_indexed_address_height.clone()
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
