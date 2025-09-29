use super::*;

mod structs;
pub mod threads;
pub use structs::*;

pub struct Server {
    pub db: Arc<DB>,
    pub event_sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub raw_event_sender: kanal::Sender<RawServerEvent>,
    pub token: WaitToken,
    pub holders: Arc<Holders>,
    pub indexer: Arc<nint_blk::Indexer>,
    pub client: Arc<nint_blk::Client>,
    pub start_time: std::time::Instant,
}

impl Server {
    pub fn new(db_path: &str) -> anyhow::Result<(kanal::Receiver<RawServerEvent>, tokio::sync::broadcast::Sender<ServerEvent>, Self)> {
        let (raw_tx, raw_rx) = kanal::unbounded();
        let (tx, _) = tokio::sync::broadcast::channel(30_000);
        let token = WaitToken::default();
        let db = Arc::new(DB::open(db_path));

        let coin = match (*BLOCKCHAIN, *NETWORK) {
            (Blockchain::Bellscoin, Network::Bellscoin) => "bellscoin",
            (Blockchain::Bellscoin, Network::Testnet) => "bellscoin-testnet",
            (Blockchain::Dogecoin, Network::Bellscoin) => "dogecoin",
            (Blockchain::Dogecoin, Network::Testnet) => "dogecoin-testnet",
            (Blockchain::DogecoinEv, Network::Bellscoin) => "dogecoinev",
            (Blockchain::DogecoinEv, Network::Testnet) => "dogecoinev-testnet",
            (Blockchain::Pepecoin, Network::Bellscoin) => "pepecoin",
            (Blockchain::Pepecoin, Network::Testnet) => "pepecoin-testnet",
            _ => "bellscoin",
        }
        .to_string();

        let coin = nint_blk::CoinType::from_str(&coin).unwrap();

        let last_height = db.last_block.get(()).unwrap_or_default();

        let client = Arc::new(nint_blk::Client::new(&URL, nint_blk::Auth::UserPass(USER.to_string(), PASS.to_string()), coin, token.clone()).unwrap());

        let indexer = nint_blk::Indexer {
            coin,
            last_block: nint_blk::BlockId {
                height: last_height as u64,
                hash: db.block_info.get(last_height).unwrap_or_default().hash.into(),
            },
            path: BLK_DIR.clone(),
            reorg_max_len: REORG_CACHE_MAX_LEN,
            token: token.clone(),
            index_dir_path: INDEX_DIR.clone(),
            client: client.clone(),
        };

        let server = Self {
            holders: Arc::new(Holders::init(&db)),
            raw_event_sender: raw_tx.clone(),
            token,
            event_sender: tx.clone(),
            indexer: Arc::new(indexer),
            db,
            client,
            start_time: std::time::Instant::now(),
        };

        Ok((raw_rx, tx, server))
    }

    pub fn load_addresses(&self, keys: impl IntoIterator<Item = FullHash>) -> anyhow::Result<AddressesFullHash> {
        let keys = keys.into_iter().collect::<HashSet<_>>();

        Ok(AddressesFullHash::new(
            self.db.fullhash_to_address.multi_get_kv(keys.iter(), false).into_iter().map(|(k, v)| (*k, v)).collect(),
        ))
    }

    pub fn generate_history_hash(prev_history_hash: sha256::Hash, history: &[(AddressTokenIdDB, HistoryValue)], addresses: &AddressesFullHash) -> anyhow::Result<sha256::Hash> {
        let current_hash = if history.is_empty() {
            *DEFAULT_HASH
        } else {
            let mut buffer = Vec::<u8>::new();

            for (address_token, action) in history {
                let rest = rest::types::History {
                    height: action.height,
                    action: rest::types::TokenAction::from_with_addresses(action.action.clone(), addresses),
                    address_token: rest::types::AddressTokenId {
                        address: addresses.get(&address_token.address),
                        id: address_token.id,
                        tick: address_token.token.into(),
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
