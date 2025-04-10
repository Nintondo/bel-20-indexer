use super::*;

mod structs;
pub mod threads;
pub use structs::*;
use threads::AddressesToLoad;

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

    pub async fn load_addresses(
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
                    &crate::rest::api::History::new(v.height, v.action.clone(), k.clone(), self)
                        .await?,
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
