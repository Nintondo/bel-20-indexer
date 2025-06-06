use super::*;
use crate::inscriptions::parser::Parser;
use crate::inscriptions::processe_data::ProcessedData;
use crate::utils::AddressesFullHash;

pub struct InscriptionIndexer {
    server: Arc<Server>,
    reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
}

#[derive(Default)]
pub struct DataToWrite {
    pub processed: Vec<ProcessedData>,
    pub block_events: Vec<ServerEvent>,
    pub history: Vec<(AddressTokenId, HistoryValue)>,
}

impl InscriptionIndexer {
    pub fn new(
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> Self {
        Self {
            reorg_cache,
            server,
        }
    }

    pub async fn handle(&self, block_height: u32, block: bellscoin::Block) -> anyhow::Result<()> {
        let mut to_write = DataToWrite::default();

        self.handle_block(&mut to_write, block_height, &block)?;

        // write/remove data from block
        for data in to_write.processed {
            data.write(&self.server.db);
        }

        for event in to_write.block_events {
            self.server.event_sender.send(event).ok();
        }

        if self.server.raw_event_sender.send(to_write.history).is_err()
            && !self.server.token.is_cancelled()
        {
            panic!("Failed to send raw event");
        }

        Ok(())
    }

    fn handle_block(
        &self,
        to_write: &mut DataToWrite,
        block_height: u32,
        block: &bellscoin::Block,
    ) -> anyhow::Result<()> {
        let current_hash = block.block_hash();

        let mut last_history_id = self.server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = self.reorg_cache.as_ref() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
            cache.lock().new_block(block_height, last_history_id);
        }

        let block_info = BlockInfo {
            created: block.header.time,
            hash: current_hash,
        };

        let prev_block_height = block_height.checked_sub(1).unwrap_or_default();
        let prev_block_proof = self
            .server
            .db
            .proof_of_history
            .get(prev_block_height)
            .unwrap_or(*DEFAULT_HASH);

        let outpoint_fullhash_to_address = block
            .txdata
            .iter()
            .flat_map(|x| &x.output)
            .filter_map(|x| {
                if x.script_pubkey.is_provably_unspendable() {
                    return None;
                }

                let fullhash = x.script_pubkey.compute_script_hash();

                bellscoin::address::Payload::from_script(&x.script_pubkey)
                    .map(|payload| (fullhash, self.server.address_decoder.encode(&payload)))
                    .ok()
            })
            .unique()
            .collect::<HashMap<_, _>>();

        to_write.processed.push(ProcessedData::Info {
            block_number: block_height,
            block_info,
        });

        let prevouts = utils::process_prevouts(
            self.server.db.clone(),
            &block.txdata,
            &mut to_write.processed,
        )?;

        to_write.processed.push(ProcessedData::FullHash {
            addresses: outpoint_fullhash_to_address
                .iter()
                .map(|(fullhash, address)| (*fullhash, address.clone()))
                .collect(),
        });

        if block_height < *START_HEIGHT {
            return Ok(());
        }

        if block.txdata.len() == 1 {
            let new_proof =
                Server::generate_history_hash(prev_block_proof, &[], &Default::default())?;

            to_write.processed.push(ProcessedData::Proof {
                block_number: block_height,
                block_proof: new_proof,
            });

            to_write.block_events.push(ServerEvent::NewBlock(
                block_height,
                new_proof,
                block.block_hash(),
            ));

            return Ok(());
        }

        if let Some(cache) = self.reorg_cache.as_ref() {
            prevouts.iter().for_each(|(key, value)| {
                cache.lock().removed_prevout(*key, value.clone());
            });
        }

        let mut token_cache = TokenCache::load(&prevouts, &self.server.db);

        let transfers_to_remove = token_cache
            .valid_transfers
            .iter()
            .map(|(key, value)| AddressLocation {
                address: value.0,
                location: *key,
            })
            .collect::<HashSet<_>>();

        let mut parser = Parser {
            token_cache: &mut token_cache,
            server: &self.server,
            reorg_cache: self.reorg_cache.clone(),
        };

        parser.parse_block(block_height, block, &prevouts, &mut to_write.processed);

        token_cache.load_tokens_data(&self.server.db)?;

        let mut fullhash_to_load = HashSet::new();

        to_write.history = token_cache
            .process_token_actions(self.reorg_cache.clone(), &self.server.holders)
            .into_iter()
            .flat_map(|action| {
                last_history_id += 1;
                let mut results: Vec<(AddressTokenId, HistoryValue)> = vec![];
                let token = action.tick();
                let recipient = action.recipient();
                fullhash_to_load.insert(recipient);
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
                    let sender = action.sender().unwrap();
                    fullhash_to_load.insert(sender);
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

                results
            })
            .collect();

        let rest_addresses: AddressesFullHash = self
            .server
            .db
            .fullhash_to_address
            .multi_get_kv(
                fullhash_to_load
                    .iter()
                    .filter(|x| !outpoint_fullhash_to_address.contains_key(x)),
                true,
            )
            .into_iter()
            .map(|(k, v)| (*k, v))
            .chain(outpoint_fullhash_to_address)
            .collect::<HashMap<_, _>>()
            .into();

        let new_proof =
            Server::generate_history_hash(prev_block_proof, &to_write.history, &rest_addresses)?;

        to_write.processed.push(ProcessedData::Proof {
            block_number: block_height,
            block_proof: new_proof,
        });

        to_write.processed.push(ProcessedData::History {
            block_number: block_height,
            last_history_id,
            history: to_write.history.clone(),
        });

        if let Some(reorg_cache) = self.reorg_cache.as_ref() {
            let mut cache = reorg_cache.lock();
            let keys = to_write.history.iter().map(|x| x.0);
            cache.extend_history(keys);
        };

        to_write.processed.push(ProcessedData::Tokens {
            metas: token_cache
                .tokens
                .into_iter()
                .map(|(k, v)| (k, TokenMetaDB::from(v)))
                .collect(),
            balances: token_cache.token_accounts.into_iter().collect(),
            transfers_to_write: token_cache
                .valid_transfers
                .into_iter()
                .map(|(location, (address, proto))| (AddressLocation { address, location }, proto))
                .collect(),
            transfers_to_remove: transfers_to_remove.into_iter().collect(),
        });

        to_write.block_events.push(ServerEvent::NewBlock(
            block_height,
            new_proof,
            block.block_hash(),
        ));

        Ok(())
    }
}

#[derive(Debug)]
pub enum ParsedInscriptionResult {
    None,
    Partials,
    Single(InscriptionTemplate),
    Many(Vec<InscriptionTemplate>),
}
