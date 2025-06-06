use crate::{
    inscriptions::load_decoder,
    utils::{address_encoder::Decoder, AddressesFullHash},
};

use super::*;

mod structs;
pub mod threads;
use bellscoin::{PublicKey, ScriptBuf};
pub use structs::*;

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

    pub fn load_addresses(
        &self,
        keys: impl IntoIterator<Item = FullHash>,
    ) -> anyhow::Result<AddressesFullHash> {
        let keys = keys.into_iter().collect::<HashSet<_>>();

        Ok(AddressesFullHash::new(
            self.db
                .fullhash_to_address
                .multi_get_kv(keys.iter(), false)
                .into_iter()
                .map(|(k, v)| (*k, v))
                .collect(),
        ))
    }

    pub fn generate_history_hash(
        prev_history_hash: sha256::Hash,
        history: &[(AddressTokenId, HistoryValue)],
        addresses: &AddressesFullHash,
    ) -> anyhow::Result<sha256::Hash> {
        let current_hash = if history.is_empty() {
            *DEFAULT_HASH
        } else {
            let mut buffer = Vec::<u8>::new();

            for (address_token, action) in history {
                let rest = rest::types::History {
                    height: action.height,
                    action: rest::types::TokenAction::from_with_addresses(
                        action.action.clone(),
                        addresses,
                    ),
                    address_token: rest::types::AddressTokenId {
                        address: addresses.get(&address_token.address),
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

    pub fn to_scripthash(&self, script_type: &str, script_str: &str) -> anyhow::Result<FullHash> {
        let Ok(pubkey) = PublicKey::from_str(script_str) else {
            return match script_type {
                "address" => self.address_to_scripthash(script_str),
                "scripthash" => hex::decode(script_str)?.try_into(),
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
}
