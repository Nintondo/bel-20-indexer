use super::*;
use crate::DEFAULT_HASH;
use crate::PASS;
use crate::USER;
use core_utils::db::tables::DB;
use core_utils::types::full_hash::FullHash;
use core_utils::types::holders::Holders;
use core_utils::types::rest::load_addresses::AddressesLoader;
use core_utils::types::rest::rest_api;
use core_utils::types::server::{RawServerEvent, ServerEvent};
use core_utils::types::structs::{AddressTokenId, HistoryValue};
use core_utils::{IsOpReturnHash, NON_STANDARD_ADDRESS, OP_RETURN_ADDRESS};
use dutils::wait_token::WaitToken;
use nintondo_dogecoin::hashes::{Hash, sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub mod threads;

pub struct Server {
    pub db: Arc<DB>,
    pub event_sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub raw_event_sender: kanal::Sender<RawServerEvent>,
    pub token: WaitToken,
    pub last_indexed_address_height: Arc<tokio::sync::RwLock<u32>>,
    pub client: electrs_client::Config,
    pub holders: Arc<Holders>,
}

impl Server {
    pub fn new(
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
            client: electrs_client::Config {
                url: URL.to_string(),
                user: USER.to_string(),
                password: PASS.to_string(),
                limit: Some(1000),
                reorgs_path: None,
            },
            holders: Arc::new(Holders::init(&db)),
            db,
            raw_event_sender: raw_tx.clone(),
            token,
            last_indexed_address_height: Arc::new(tokio::sync::RwLock::new(0)),
            event_sender: tx.clone(),
        };

        Ok((raw_rx, tx, server))
    }

    pub fn generate_history_hash(
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
        keys: impl IntoIterator<Item = FullHash>,
        height: u32,
    ) -> anyhow::Result<HashMap<FullHash, String>> {
        let mut counter = 0;
        while *self.last_indexed_address_height.read().await < height {
            if counter > 100 {
                anyhow::bail!("Something went wrong with the addresses");
            }

            counter += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

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
