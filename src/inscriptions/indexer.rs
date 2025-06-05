use super::*;
use crate::inscriptions::parser::Parser;
use crate::inscriptions::processe_data::ProcessedData;

pub struct Indexer;

impl Indexer {
    pub async fn handle(
        block_height: u32,
        block: bellscoin::Block,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let mut data_to_write: Vec<ProcessedData> = vec![];
        let mut block_events: Vec<ServerEvent> = vec![];
        let mut history = vec![];

        Self::handle_block(
            &mut data_to_write,
            &mut block_events,
            &mut history,
            block_height,
            &block,
            &server,
            reorg_cache,
        )?;

        // write/remove data from block
        for data in data_to_write {
            data.write(&server.db);
        }

        *server.last_indexed_address_height.write().await = block_height;

        for event in block_events {
            server.event_sender.send(event).ok();
        }

        if server.raw_event_sender.send(history).is_err() && !server.token.is_cancelled() {
            panic!("Failed to send raw event");
        }

        Ok(())
    }

    fn handle_block(
        data_to_write: &mut Vec<ProcessedData>,
        block_events: &mut Vec<ServerEvent>,
        history: &mut Vec<(AddressTokenId, HistoryValue)>,
        block_height: u32,
        block: &bellscoin::Block,
        server: &Server,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let current_hash = block.block_hash();

        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
            cache.lock().new_block(block_height, last_history_id);
        }

        let block_info = BlockInfo {
            created: block.header.time,
            hash: current_hash,
        };

        let created = block.header.time;
        let prev_block_height = block_height.checked_sub(1).unwrap_or_default();
        let prev_block_proof = server
            .db
            .proof_of_history
            .get(prev_block_height)
            .unwrap_or(*DEFAULT_HASH);

        let outpoint_fullhash_to_address = block
            .txdata
            .iter()
            .flat_map(|x| &x.output)
            .filter_map(|x| {
                let fullhash = x.script_pubkey.compute_script_hash();
                let payload = bellscoin::address::Payload::from_script(&x.script_pubkey);
                if x.script_pubkey.is_op_return() || payload.is_err() {
                    return None;
                }
                Some((fullhash, server.address_decoder.encode(&payload.unwrap())))
            })
            .collect::<HashMap<_, _>>();

        data_to_write.push(ProcessedData::Info {
            block_number: block_height,
            block_info,
        });

        let prevouts = utils::process_prevouts(server.db.clone(), &block.txdata, data_to_write)?;

        data_to_write.push(ProcessedData::FullHash {
            addresses: outpoint_fullhash_to_address
                .iter()
                .map(|(fullhash, address)| (*fullhash, address.clone()))
                .collect(),
        });

        if block_height < *START_HEIGHT {
            return Ok(());
        }

        if block.txdata.len() == 1 {
            let new_proof = Server::generate_history_hash(prev_block_proof, &[], &HashMap::new())?;

            data_to_write.push(ProcessedData::Proof {
                block_number: block_height,
                block_proof: new_proof,
            });

            block_events.push(ServerEvent::NewBlock(
                block_height,
                new_proof,
                block.block_hash(),
            ));

            return Ok(());
        }

        if let Some(cache) = reorg_cache.as_ref() {
            prevouts.iter().for_each(|(key, value)| {
                cache.lock().removed_prevout(*key, value.clone());
            });
        }

        let (mut token_cache, transfers_to_remove) = TokenCache::new(&prevouts, &server.db);

        Parser::parse_block(
            server,
            data_to_write,
            block_height,
            created,
            &block.txdata,
            &prevouts,
            &mut token_cache,
            reorg_cache.clone(),
        );

        token_cache.load_tokens_data(&server.db)?;

        let mut fullhash_to_load = HashSet::new();

        *history = token_cache
            .process_token_actions(reorg_cache.clone(), &server.holders)
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
                    let sender = action
                        .sender()
                        .expect("Should be in here with the Send action");
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

        let mut rest_addresses = server
            .db
            .fullhash_to_address
            .multi_get(fullhash_to_load.iter())
            .into_iter()
            .zip(fullhash_to_load)
            .map(|(v, k)| {
                if k.is_op_return_hash() {
                    return (k, OP_RETURN_ADDRESS.to_string());
                }
                if v.is_none() {
                    return (k, NON_STANDARD_ADDRESS.to_string());
                }
                (k, v.unwrap())
            })
            .collect::<HashMap<_, _>>();
        rest_addresses.extend(outpoint_fullhash_to_address);

        let new_proof = Server::generate_history_hash(prev_block_proof, history, &rest_addresses)?;

        data_to_write.push(ProcessedData::Proof {
            block_number: block_height,
            block_proof: new_proof,
        });

        data_to_write.push(ProcessedData::History {
            block_number: block_height,
            last_history_id,
            history: history.clone(),
        });

        if let Some(reorg_cache) = reorg_cache.as_ref() {
            let mut cache = reorg_cache.lock();
            history
                .iter()
                .for_each(|(k, _)| cache.added_history(k.clone()));
        };

        data_to_write.push(ProcessedData::Tokens {
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

        block_events.push(ServerEvent::NewBlock(
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
