use std::collections::VecDeque;

use super::*;
use crate::inscriptions::types::{
    HistoryLocation, Outpoint, ParsedTokenHistory, ParsedTokenHistoryData, TokenHistory,
    TokenHistoryData,
};
use nintondo_dogecoin::{Address, hashes::serde_macros::serde_details::SerdeHash};

pub struct InitialIndexer {}

pub struct TxidN(pub [u8; 32]);

impl From<TxidN> for Txid {
    fn from(value: TxidN) -> Self {
        Txid::from_slice(&value.0).expect("Unexpected txid")
    }
}

impl From<Outpoint> for OutPoint {
    fn from(value: Outpoint) -> Self {
        Self {
            txid: TxidN(value.txid).into(),
            vout: value.vout,
        }
    }
}

impl From<HistoryLocation> for Location {
    fn from(value: HistoryLocation) -> Self {
        Location {
            outpoint: value.outpoint.into(),
            offset: value.offset,
        }
    }
}

impl InitialIndexer {
    fn parse_block(
        height: u32,
        created: u32,
        ths: &[ParsedTokenHistory],
        token_cache: &mut TokenCache,
    ) {
        let mut inscription_idx = 0;
        for th in ths {
            let location = th.to_location.into();
            let owner = th.to.compute_script_hash();
            let txid = TxidN(th.from_location.outpoint.txid).into();
            let vout = th.from_location.outpoint.vout;

            match th.token {
                inscriptions::types::ParsedTokenActionRest::Deploy {
                    tick,
                    max,
                    lim,
                    dec,
                } => {
                    token_cache.token_actions.push(TokenAction::Deploy {
                        genesis: InscriptionId {
                            txid,
                            index: inscription_idx,
                        },
                        proto: DeployProtoDB {
                            tick,
                            max,
                            lim,
                            dec,
                            supply: Fixed128::ZERO,
                            transfer_count: 0,
                            mint_count: 0,
                            height,
                            created,
                            deployer: th.from.compute_script_hash(),
                            transactions: 1,
                        },
                        owner,
                    });
                    inscription_idx += 1;
                }
                inscriptions::types::ParsedTokenActionRest::Mint { tick, amt } => {
                    token_cache.token_actions.push(TokenAction::Mint {
                        owner,
                        proto: MintProto::Bel20 { tick, amt },
                        txid,
                        vout,
                    })
                }
                inscriptions::types::ParsedTokenActionRest::DeployTransfer { tick, amt } => {
                    token_cache.token_actions.push(TokenAction::Transfer {
                        location,
                        owner,
                        proto: TransferProto::Bel20 { tick, amt },
                        txid,
                        vout,
                    });

                    token_cache
                        .all_transfers
                        .insert(location, TransferProtoDB { tick, amt, height });
                }
                inscriptions::types::ParsedTokenActionRest::SpentTransfer { .. } => {
                    if th.leaked {
                        token_cache.burned_transfer(location, txid, vout); // todo use burned txid and vout
                    } else {
                        token_cache.transferred(location, owner, txid, vout);
                    }
                }
            };
        }
    }

    pub async fn handle_batch(
        token_history_data: Vec<ParsedTokenHistoryData>,
        server: &Server,
    ) -> anyhow::Result<()> {
        // used to get all data from db and generate keys
        let batch_cache = BatchCache::load_cache(server, &token_history_data);

        // generate shared cache for updates
        let mut shared_cache = batch_cache.shared_cache();

        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        // store all proofs for batch
        let mut block_height_to_history = HashMap::<u32, BlockHistory>::new();

        // last proof from db
        let mut prev_proof = server
            .db
            .last_block
            .get(())
            .and_then(|height| server.db.proof_of_history.get(height))
            .unwrap_or(*DEFAULT_HASH);

        let mut full_hash_to_address = HashMap::<FullHash, String>::new();

        for block in token_history_data {
            let block_cache = batch_cache
                .get_block_cache(block.block_info.height)
                .expect("Block cache must exist, generated above");

            let mut token_cache = shared_cache.generate_block_token_cache(&block_cache);
            let history = token_cache.process_token_actions(None, &server.holders);
            shared_cache.update_cache(token_cache);

            let mut block_history: Vec<(AddressTokenId, HistoryValue)> = Vec::new();
            for action in history {
                last_history_id += 1;

                let token = action.tick();
                let recipient = action.recipient();
                let key = AddressTokenId {
                    address: recipient,
                    token,
                    id: last_history_id,
                };
                let db_action = TokenHistoryDB::from_token_history(action.clone());
                if let TokenHistoryDB::Send {
                    amt, txid, vout, ..
                } = db_action
                {
                    let sender = action
                        .sender()
                        .expect("Should be in here with the Send action");
                    last_history_id += 1;
                    block_history.push((
                        AddressTokenId {
                            address: sender,
                            token,
                            id: last_history_id,
                        },
                        HistoryValue {
                            height: block.block_info.height,
                            action: db_action,
                        },
                    ));
                    block_history.push((
                        key,
                        HistoryValue {
                            height: block.block_info.height,
                            action: TokenHistoryDB::Receive {
                                amt,
                                sender,
                                txid,
                                vout,
                            },
                        },
                    ))
                } else {
                    block_history.push((
                        key,
                        HistoryValue {
                            action: db_action,
                            height: block.block_info.height,
                        },
                    ));
                }
            }

            let new_block_proof = Server::generate_history_hash(
                prev_proof,
                &block_history,
                &block_cache.full_hash_to_address,
            )
            .expect("Must generate history proof");

            full_hash_to_address.extend(block_cache.full_hash_to_address);

            block_height_to_history.insert(
                block.block_info.height,
                BlockHistory {
                    block_hash: block.block_info.block_hash,
                    proof: new_block_proof,
                    history: block_history,
                },
            );
            prev_proof = new_block_proof;
        }

        let last_block_height = block_height_to_history
            .keys()
            .sorted()
            .next_back()
            .cloned()
            .expect("Last block height must exist in batch");

        // write/rewrite tokens
        server.db.token_to_meta.extend(
            shared_cache
                .token_to_meta
                .into_iter()
                .map(|(tick, proto)| (tick, TokenMetaDB::from(proto))),
        );

        // write/rewrite address token balance
        server
            .db
            .address_token_to_balance
            .extend(shared_cache.account_to_balance);

        // remove spent token transfers
        server
            .db
            .address_location_to_transfer
            .remove_batch(shared_cache.transfer_to_remove.into_iter());

        // write new address token transfers
        server
            .db
            .address_location_to_transfer
            .extend(shared_cache.address_location_to_transfer);

        for (height, block_history) in &block_height_to_history {
            let history_idx = block_history
                .history
                .iter()
                .map(|(address_token_id, _)| address_token_id.clone())
                .sorted_unstable_by_key(|address_token_id| address_token_id.id)
                .collect_vec();

            server.db.block_events.set(height, history_idx);

            let outpoint_idx = block_history
                .history
                .iter()
                .map(|(address_token_id, history)| {
                    (history.action.outpoint(), address_token_id.clone())
                });

            server.db.outpoint_to_event.extend(outpoint_idx);

            server
                .db
                .address_token_to_history
                .extend(block_history.history.clone());
        }

        //write history proofs
        server.db.proof_of_history.extend(
            block_height_to_history
                .iter()
                .map(|(height, history)| (height, history.proof)),
        );

        server.db.last_history_id.set((), last_history_id);
        server.db.last_block.set((), last_block_height);

        // write all addresses
        server.db.fullhash_to_address.extend(full_hash_to_address);

        *server.last_indexed_address_height.write().await = last_block_height;

        for (block_height, block_history) in block_height_to_history {
            server
                .event_sender
                .send(ServerEvent::NewBlock(
                    block_height,
                    block_history.proof,
                    block_history.block_hash,
                ))
                .ok();
            if server.raw_event_sender.send(block_history.history).is_err()
                && !server.token.is_cancelled()
            {
                panic!("Failed to send raw event");
            }
        }

        Ok(())
    }

    pub async fn handle(
        token_history_data: ParsedTokenHistoryData,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let block_info = token_history_data.block_info;
        let block_height = block_info.height;
        let current_hash = block_info.block_hash;
        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
            cache.lock().new_block(block_info.into(), last_history_id);
        }

        server.db.block_hashes.set(block_height, current_hash);

        if reorg_cache.is_some() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let block = token_history_data;
        let created = block.block_info.created;

        match server.addr_tx.send(server::threads::AddressesToLoad {
            height: block_height,
            addresses: block
                .inscriptions
                .iter()
                .flat_map(|x| vec![x.from.clone(), x.to.clone()])
                .collect(),
        }) {
            Ok(_) => {}
            _ => {
                if !server.token.is_cancelled() {
                    panic!("Failed to send addresses to load");
                }
            }
        }

        if block_height < *START_HEIGHT {
            server.db.last_block.set((), block_height);
            return Ok(());
        }

        if block.inscriptions.is_empty() {
            server.db.last_block.set((), block_height);
            return server.new_hash(block_height, current_hash, &[]).await;
        }

        let mut token_cache = TokenCache::default();

        token_cache.valid_transfers.extend(
            server.db.load_transfers(
                block
                    .inscriptions
                    .iter()
                    .filter(|x| {
                        matches!(x.token, types::ParsedTokenActionRest::SpentTransfer { .. })
                    })
                    .map(|k| AddressLocation {
                        address: k.to.compute_script_hash(),
                        location: Location {
                            outpoint: k.to_location.outpoint.into(),
                            offset: 0,
                        },
                    })
                    .collect(),
            ),
        );

        Self::parse_block(block_height, created, &block.inscriptions, &mut token_cache);

        token_cache.load_tokens_data(&server.db)?;

        let history = token_cache
            .process_token_actions(reorg_cache.clone(), &server.holders)
            .into_iter()
            .flat_map(|action| {
                last_history_id += 1;
                let mut results: Vec<(AddressTokenId, HistoryValue)> = vec![];
                let token = action.tick();
                let recipient = action.recipient();
                let key = AddressTokenId {
                    address: recipient,
                    token,
                    id: last_history_id,
                };
                let db_action = TokenHistoryDB::from_token_history(action.clone());
                if let TokenHistoryDB::Send {
                    amt, txid, vout, ..
                } = db_action
                {
                    let sender = action
                        .sender()
                        .expect("Should be in here with the Send action");
                    last_history_id += 1;
                    results.extend([
                        (
                            AddressTokenId {
                                address: sender,
                                token,
                                id: last_history_id,
                            },
                            HistoryValue {
                                height: block_height,
                                action: db_action,
                            },
                        ),
                        (
                            key,
                            HistoryValue {
                                height: block_height,
                                action: TokenHistoryDB::Receive {
                                    amt,
                                    sender,
                                    txid,
                                    vout,
                                },
                            },
                        ),
                    ])
                } else {
                    results.push((
                        key,
                        HistoryValue {
                            action: db_action,
                            height: block_height,
                        },
                    ));
                }
                match server.raw_event_sender.send(results.clone()) {
                    Ok(_) => {}
                    _ => {
                        if !server.token.is_cancelled() {
                            panic!("Failed to send raw event");
                        }
                    }
                }
                results
            })
            .collect_vec();

        if let Some(reorg_cache) = reorg_cache.as_ref() {
            let mut cache = reorg_cache.lock();
            history
                .iter()
                .for_each(|(k, _)| cache.added_history(k.clone()));
        };

        {
            let new_keys = history
                .iter()
                .map(|x| x.0.clone())
                .sorted_unstable_by_key(|x| x.id)
                .collect_vec();
            server.db.block_events.set(block_height, new_keys);

            let keys = history.iter().map(|x| (x.1.action.outpoint(), x.0.clone()));
            server.db.outpoint_to_event.extend(keys)
        }

        server
            .new_hash(block_height, current_hash, &history)
            .await?;

        server.db.address_token_to_history.extend(history);

        token_cache.write_token_data(server.db.clone()).await?;
        token_cache.write_valid_transfers(&server.db)?;

        server.db.last_block.set((), block_height);
        server.db.last_history_id.set((), last_history_id);
        Ok(())
    }
}

pub struct BatchCache {
    // cache for all batch of blocks
    pub token_to_meta: HashMap<LowerCaseTick, TokenMeta>,
    pub account_to_balance: HashMap<AddressToken, TokenBalance>,
    pub address_location_to_transfer: HashMap<AddressLocation, TransferProtoDB>,
    // keys for block to get cached data
    pub block_number_to_block_cache: HashMap<u32, BlockCache>,
}

#[derive(Default)]
pub struct SharedBatchCache {
    pub token_to_meta: HashMap<LowerCaseTick, TokenMeta>,
    pub account_to_balance: HashMap<AddressToken, TokenBalance>,
    pub address_location_to_transfer: HashMap<AddressLocation, TransferProtoDB>,
    pub transfer_to_remove: HashSet<AddressLocation>,
}

impl SharedBatchCache {
    pub fn generate_block_token_cache(&mut self, block_cache: &BlockCache) -> TokenCache {
        let tokens: HashMap<_, _> = self
            .token_to_meta
            .iter()
            .filter(|(key, _)| block_cache.tokens.contains(key))
            .map(|(tick, meta)| (tick.clone(), meta.clone()))
            .collect();

        let token_accounts: HashMap<_, _> = self
            .account_to_balance
            .iter()
            .filter(|(key, _)| block_cache.address_token.contains(key))
            .map(|(address, balance)| (address.clone(), balance.clone()))
            .collect();

        let mut valid_transfers = BTreeMap::<_, _>::new();
        for key in &block_cache.address_transfer_location {
            let Some(data) = self.address_location_to_transfer.remove(key) else {
                continue;
            };
            self.transfer_to_remove.insert(key.clone());
            valid_transfers.insert(key.location, (key.address, data.clone()));
        }

        let mut token_cache = block_cache.token_cache.clone();
        token_cache.tokens = tokens;
        token_cache.token_accounts = token_accounts;
        token_cache.valid_transfers = valid_transfers;

        token_cache
    }

    pub fn update_cache(&mut self, token_cache: TokenCache) {
        // update tokens deploys from block
        self.token_to_meta.extend(token_cache.tokens);
        // update address balance from block
        self.account_to_balance.extend(token_cache.token_accounts);

        // return not spent transfers from block
        self.address_location_to_transfer.extend(
            token_cache
                .valid_transfers
                .into_iter()
                .map(|(location, (address, proto))| (AddressLocation { address, location }, proto)),
        );
    }
}

#[derive(Default, Clone)]
pub struct BlockCache {
    pub full_hash_to_address: HashMap<FullHash, String>,
    pub tokens: HashSet<LowerCaseTick>,
    pub address_token: HashSet<AddressToken>,
    pub address_transfer_location: HashSet<AddressLocation>,
    pub token_cache: TokenCache,
}

impl BatchCache {
    pub fn load_cache(server: &Server, history: &[ParsedTokenHistoryData]) -> Self {
        let mut block_number_to_block_cache = HashMap::<u32, BlockCache>::new();

        for block in history {
            let mut block_cache = BlockCache::default();
            let mut inscription_idx = 0;
            let mut temp_addresses = HashSet::new();

            for inscription in &block.inscriptions {
                // got txid:vout where token action was happened
                let txid = Txid::from_slice(&inscription.from_location.outpoint.txid).unwrap();
                let vout = inscription.from_location.outpoint.vout;

                match inscription.token {
                    types::ParsedTokenActionRest::Mint { tick, amt } if !inscription.leaked => {
                        let token: LowerCaseTick = tick.into();
                        let account = AddressToken {
                            address: inscription.to.compute_script_hash(),
                            token: token.clone(),
                        };

                        block_cache
                            .token_cache
                            .token_actions
                            .push(TokenAction::Mint {
                                owner: account.address,
                                proto: MintProto::Bel20 { tick, amt },
                                txid,
                                vout,
                            });

                        block_cache.tokens.insert(token);
                        block_cache.address_token.insert(account);
                        temp_addresses.insert(inscription.from.clone());
                        temp_addresses.insert(inscription.to.clone());
                    }
                    types::ParsedTokenActionRest::DeployTransfer { tick, amt }
                        if !inscription.leaked =>
                    {
                        let token: LowerCaseTick = tick.into();
                        let account = AddressToken {
                            address: inscription.to.compute_script_hash(),
                            token: token.clone(),
                        };
                        let address_location = AddressLocation {
                            address: account.address,
                            location: inscription.to_location.into(),
                        };

                        block_cache
                            .token_cache
                            .token_actions
                            .push(TokenAction::Transfer {
                                location: address_location.location,
                                owner: address_location.address,
                                proto: TransferProto::Bel20 { tick, amt },
                                txid,
                                vout,
                            });

                        block_cache.token_cache.all_transfers.insert(
                            address_location.location,
                            TransferProtoDB {
                                tick,
                                amt,
                                height: block.block_info.height,
                            },
                        );

                        block_cache.tokens.insert(token);
                        block_cache
                            .address_transfer_location
                            .insert(address_location);
                        block_cache.address_token.insert(account);
                        temp_addresses.insert(inscription.to.clone());
                        temp_addresses.insert(inscription.from.clone());
                    }
                    types::ParsedTokenActionRest::SpentTransfer { tick, .. } => {
                        let token: LowerCaseTick = tick.into();
                        let account = AddressToken {
                            address: inscription.from.compute_script_hash(),
                            token: token.clone(),
                        };

                        if inscription.leaked {
                            block_cache
                                .token_cache
                                .token_actions
                                .push(TokenAction::Transferred {
                                    transfer_location: inscription.from_location.into(),
                                    recipient: None,
                                    txid,
                                    vout,
                                });
                        } else {
                            block_cache
                                .token_cache
                                .token_actions
                                .push(TokenAction::Transferred {
                                    transfer_location: inscription.from_location.into(),
                                    recipient: Some(inscription.to.compute_script_hash()),
                                    txid,
                                    vout,
                                });
                        }

                        temp_addresses.insert(inscription.to.clone());
                        temp_addresses.insert(inscription.from.clone());
                        block_cache.tokens.insert(token.clone());
                        block_cache
                            .address_transfer_location
                            .insert(AddressLocation {
                                address: account.address,
                                location: inscription.from_location.into(),
                            });
                        block_cache.address_token.insert(account);
                        block_cache.address_token.insert(AddressToken {
                            address: inscription.to.compute_script_hash(),
                            token,
                        });
                    }
                    types::ParsedTokenActionRest::Deploy {
                        tick,
                        max,
                        lim,
                        dec,
                    } if !inscription.leaked => {
                        block_cache
                            .token_cache
                            .token_actions
                            .push(TokenAction::Deploy {
                                genesis: InscriptionId {
                                    txid,
                                    index: inscription_idx,
                                },
                                proto: DeployProtoDB {
                                    tick,
                                    max,
                                    lim,
                                    dec,
                                    supply: Fixed128::ZERO,
                                    transfer_count: 0,
                                    mint_count: 0,
                                    height: block.block_info.height,
                                    created: block.block_info.created,
                                    deployer: inscription.to.compute_script_hash(),
                                    transactions: 1,
                                },
                                owner: inscription.to.compute_script_hash(),
                            });
                        temp_addresses.insert(inscription.to.clone());
                        temp_addresses.insert(inscription.from.clone());
                        inscription_idx += 1;
                    }
                    _ => continue,
                }
            }

            block_cache.full_hash_to_address = temp_addresses
                .into_iter()
                .flat_map(|x| match x {
                    types::ParsedTokenAddress::Standard(str) => str
                        .to_address_str(*NETWORK)
                        .map(|v| (str.compute_script_hash(), v)),
                    types::ParsedTokenAddress::NonStandard(_) => None,
                })
                .collect();

            block_number_to_block_cache.insert(block.block_info.height, block_cache);
        }

        let ticks: HashSet<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.tokens.clone())
            .collect();

        let address_token: HashSet<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.address_token.clone())
            .collect();

        let address_transfer_location: HashSet<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.address_transfer_location.clone())
            .collect();

        let token_to_meta = server
            .db
            .token_to_meta
            .multi_get(ticks.iter())
            .into_iter()
            .zip(ticks)
            .filter_map(|(token_meta, token)| token_meta.map(|meta| (token, TokenMeta::from(meta))))
            .collect::<HashMap<_, _>>();

        let account_to_balance = server
            .db
            .address_token_to_balance
            .multi_get(address_token.iter())
            .into_iter()
            .zip(address_token)
            .flat_map(|(token_balance, address_token)| {
                token_balance.map(|balance| (address_token, balance))
            })
            .collect::<HashMap<_, _>>();

        let address_location_to_transfer = server
            .db
            .address_location_to_transfer
            .multi_get(address_transfer_location.iter())
            .into_iter()
            .zip(address_transfer_location)
            .flat_map(|(transfer, address_location)| {
                transfer.map(|transfer| (address_location, transfer))
            })
            .collect::<HashMap<_, _>>();

        Self {
            token_to_meta,
            account_to_balance,
            address_location_to_transfer,
            block_number_to_block_cache,
        }
    }

    pub fn shared_cache(&self) -> SharedBatchCache {
        SharedBatchCache {
            token_to_meta: self.token_to_meta.clone(),
            account_to_balance: self.account_to_balance.clone(),
            address_location_to_transfer: self.address_location_to_transfer.clone(),
            ..Default::default()
        }
    }

    pub fn get_block_cache(&self, block_number: u32) -> Option<BlockCache> {
        self.block_number_to_block_cache.get(&block_number).cloned()
    }
}

struct BlockHistory {
    pub block_hash: BlockHash,
    pub proof: sha256::Hash,
    pub history: Vec<(AddressTokenId, HistoryValue)>,
}
