use std::default::Default;

use super::*;
use crate::inscriptions::types::{HistoryLocation, Outpoint, ParsedTokenHistoryData};

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
    pub async fn handle_batch(
        token_history_data: Vec<ParsedTokenHistoryData>,
        server: &Server,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) {
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

        // store only standard addresses
        let mut full_hash_to_address = HashMap::<FullHash, String>::new();

        for block in token_history_data {
            if let Some(cache) = reorg_cache.as_ref() {
                cache
                    .lock()
                    .new_block(block.block_info.into(), last_history_id);

                debug!(
                    "Syncing block: {} ({})",
                    block.block_info.block_hash, block.block_info.height
                );
            }

            let block_cache = batch_cache
                .get_block_cache(block.block_info.height)
                .expect("Block cache must exist, generated above");

            let mut token_cache = shared_cache.generate_block_token_cache(&block_cache);
            let history = token_cache.process_token_actions(reorg_cache.clone(), &server.holders);
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

            let rest_addresses = block_cache
                .addresses
                .into_iter()
                .flat_map(|x| match x {
                    types::ParsedTokenAddress::Standard(script) => {
                        script.to_address_str(*NETWORK).map(|v| {
                            let full_hash = script.compute_script_hash();
                            let str_address = if script.is_op_return() {
                                OP_RETURN_ADDRESS.to_string()
                            } else {
                                full_hash_to_address.insert(full_hash, v.clone());
                                v
                            };
                            (full_hash, str_address)
                        })
                    }
                    types::ParsedTokenAddress::NonStandard(full_hash) => {
                        Some((full_hash, NON_STANDARD_ADDRESS.to_string()))
                    }
                })
                .collect();

            let new_block_proof =
                Server::generate_history_hash(prev_proof, &block_history, &rest_addresses)
                    .expect("Must generate history proof");

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

        // write all addresses
        server.db.fullhash_to_address.extend(full_hash_to_address);

        for (height, block_history) in block_height_to_history
            .iter()
            .sorted_by_key(|(block_number, _)| *block_number)
        {
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
        *server.last_indexed_address_height.write().await = last_block_height;

        for (block_height, block_history) in block_height_to_history
            .into_iter()
            .sorted_by_key(|(height, _)| *height)
        {
            if let Some(reorg_cache) = reorg_cache.as_ref() {
                let mut cache = reorg_cache.lock();
                block_history
                    .history
                    .iter()
                    .for_each(|(k, _)| cache.added_history(k.clone()));
            };

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
        let tokens: HashMap<_, _> = block_cache
            .tokens
            .iter()
            .flat_map(|tick| {
                self.token_to_meta
                    .remove(tick)
                    .map(|meta| (tick.clone(), meta))
            })
            .collect();

        let token_accounts: HashMap<_, _> = block_cache
            .address_token
            .iter()
            .flat_map(|address_token| {
                self.account_to_balance
                    .remove(address_token)
                    .map(|balance| (address_token.clone(), balance))
            })
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
    pub addresses: HashSet<types::ParsedTokenAddress>,
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
            let mut addresses = HashSet::new();

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
                        addresses.insert(inscription.from.clone());
                        addresses.insert(inscription.to.clone());
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
                        addresses.insert(inscription.to.clone());
                        addresses.insert(inscription.from.clone());
                    }
                    types::ParsedTokenActionRest::SpentTransfer { outpoint, tick, .. } => {
                        let token: LowerCaseTick = tick.into();
                        let account = AddressToken {
                            address: inscription.from.compute_script_hash(),
                            token: token.clone(),
                        };

                        if inscription.leaked {
                            let leaked_outpoint = outpoint.expect("Must exist leaked outpoint");
                            block_cache
                                .token_cache
                                .token_actions
                                .push(TokenAction::Transferred {
                                    transfer_location: inscription.from_location.into(),
                                    recipient: account.address,
                                    txid: leaked_outpoint.txid,
                                    vout: leaked_outpoint.vout,
                                });
                        } else {
                            block_cache
                                .token_cache
                                .token_actions
                                .push(TokenAction::Transferred {
                                    transfer_location: inscription.from_location.into(),
                                    recipient: inscription.to.compute_script_hash(),
                                    txid,
                                    vout,
                                });
                        }

                        addresses.insert(inscription.to.clone());
                        addresses.insert(inscription.from.clone());
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
                        addresses.insert(inscription.to.clone());
                        addresses.insert(inscription.from.clone());
                        inscription_idx += 1;
                    }
                    _ => continue,
                }
            }

            block_cache.addresses = addresses;
            block_number_to_block_cache.insert(block.block_info.height, block_cache);
        }

        let ticks: Vec<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.tokens.clone().into_iter())
            .sorted()
            .unique()
            .collect();

        let address_token: Vec<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.address_token.clone().into_iter())
            .sorted()
            .unique()
            .collect();

        let address_transfer_location: Vec<_> = block_number_to_block_cache
            .values()
            .flat_map(|x| x.address_transfer_location.clone().into_iter())
            .sorted()
            .unique()
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
