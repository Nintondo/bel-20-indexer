use super::{
    structs::{Part, Partials},
    *,
};

pub struct ParseInscription<'a> {
    tx: &'a Transaction,
    input_index: u32,
    inputs_cum: &'a [u64],
    partials: &'a Partials,
}

pub struct InitialIndexer;

impl InitialIndexer {
    fn load_partials(server: &Server, outpoints: Vec<OutPoint>) -> HashMap<OutPoint, Partials> {
        server
            .db
            .outpoint_to_partials
            .multi_get(outpoints.iter())
            .into_iter()
            .zip(outpoints)
            .filter_map(|(partials, outpoint)| partials.map(|partials| (outpoint, partials)))
            .collect()
    }

    fn load_inscription_outpoint_to_offsets(
        server: &Server,
        outpoints: Vec<OutPoint>,
    ) -> HashMap<OutPoint, HashSet<u64>> {
        server
            .db
            .outpoint_to_inscription_offsets
            .multi_get(outpoints.iter())
            .into_iter()
            .zip(outpoints)
            .filter_map(|(offsets, outpoint)| offsets.map(|offsets| (outpoint, offsets)))
            .collect()
    }

    fn parse_block(
        server: &Server,
        data_to_write: &mut Vec<Box<dyn ProcessedData>>,
        height: u32,
        created: u32,
        txs: &[Transaction],
        prevouts: &HashMap<OutPoint, TxOut>,
        token_cache: &mut TokenCache,
    ) {
        let take_multiple_input = if height as usize >= *JUBILEE_HEIGHT {
            usize::MAX
        } else {
            1
        };

        let coinbase_value = txs
            .first()
            .map(|coinbase| {
                coinbase
                    .output
                    .iter()
                    .map(|outpoint| outpoint.value)
                    .sum::<u64>()
            })
            .unwrap_or_default();

        // Hold inscription's partials from db and new in the block
        let mut outpoint_to_partials =
            Self::load_partials(server, prevouts.keys().cloned().collect());

        // Hold inscription's partials to remove from db
        let partials_to_remove: Vec<_> = outpoint_to_partials.keys().cloned().collect();

        let mut inscription_outpoint_to_offsets =
            Self::load_inscription_outpoint_to_offsets(server, prevouts.keys().cloned().collect());

        for tx in txs {
            if tx.is_coin_base() {
                continue;
            }

            let mut inscription_index_in_tx = 0;
            let txid = tx.txid();

            let inputs_cum = InscriptionSearcher::calc_offsets(tx, prevouts)
                .expect("failed to find all txos to calculate offsets");

            for (input_index, txin) in tx.input.iter().enumerate() {
                // handle inscription moves
                if let Some(inscription_offsets) =
                    inscription_outpoint_to_offsets.remove(&txin.previous_output)
                {
                    for inscription_offset in inscription_offsets {
                        let old_location = Location {
                            outpoint: txin.previous_output,
                            offset: inscription_offset,
                        };

                        let is_token_transfer_move =
                            token_cache.all_transfers.contains_key(&old_location);

                        let offset = inputs_cum.get(input_index).map(|x| *x + inscription_offset);
                        match InscriptionSearcher::get_output_index_by_input(offset, &tx.output) {
                            Ok((new_vout, new_offset)) => {
                                let new_outpoint = OutPoint {
                                    txid,
                                    vout: new_vout,
                                };

                                inscription_outpoint_to_offsets
                                    .entry(new_outpoint)
                                    .or_default()
                                    .insert(new_offset);

                                // handle move of token transfer
                                if is_token_transfer_move {
                                    if tx.output[new_vout as usize].script_pubkey.is_op_return() {
                                        token_cache.burned_transfer(old_location, txid, new_vout);
                                    } else {
                                        let owner = tx.output[new_vout as usize]
                                            .script_pubkey
                                            .compute_script_hash();
                                        token_cache.transferred(
                                            old_location,
                                            owner,
                                            txid,
                                            new_vout,
                                        );
                                    };
                                }
                            }
                            _ => {
                                // handle leaked move of token transfer
                                if is_token_transfer_move {
                                    // because of token protocol leaked token amount
                                    // comeback to owner
                                    let recipient = prevouts
                                        .get(&txin.previous_output)
                                        .expect("Owner of token transfer must exist")
                                        .script_pubkey
                                        .compute_script_hash();
                                    token_cache.transferred(old_location, recipient, tx.txid(), 0);
                                }

                                todo!() // TODO handle leak
                            }
                        }
                    }
                }

                // handle inscription creation
                if input_index < take_multiple_input {
                    let mut partials = outpoint_to_partials
                        .remove(&txin.previous_output)
                        .unwrap_or(Partials {
                            genesis_txid: txid,
                            inscription_index: 0,
                            parts: vec![],
                        });

                    let part = if let Some(tapscript) = txin.witness.tapscript() {
                        Part {
                            is_tapscript: true,
                            script_buffer: tapscript.to_bytes(),
                        }
                    } else {
                        Part {
                            is_tapscript: false,
                            script_buffer: txin.script_sig.to_bytes(),
                        }
                    };

                    partials.parts.push(part);

                    let parsed_result = Self::parse_inscription(ParseInscription {
                        tx,
                        input_index: input_index as u32,
                        inputs_cum: &inputs_cum,
                        partials: &partials,
                    });

                    let inscription_templates = match parsed_result {
                        ParsedInscriptionResult::None => continue,
                        ParsedInscriptionResult::Partials => {
                            if partials.genesis_txid == txid {
                                partials.inscription_index = inscription_index_in_tx;
                                inscription_index_in_tx += 1;
                            }
                            outpoint_to_partials.insert(txin.previous_output, partials);
                            continue;
                        }
                        ParsedInscriptionResult::Single(mut inscription_template) => {
                            if partials.genesis_txid == txid {
                                inscription_template.genesis.index = inscription_index_in_tx;
                                inscription_index_in_tx += 1;
                            }
                            vec![inscription_template]
                        }
                        ParsedInscriptionResult::Many(mut inscription_templates) => {
                            if partials.genesis_txid == txid {
                                inscription_templates
                                    .iter_mut()
                                    .for_each(|inscription_template| {
                                        inscription_template.genesis.index =
                                            inscription_index_in_tx;
                                        inscription_index_in_tx += 1;
                                    });
                            }

                            inscription_templates
                        }
                    };

                    for inscription_template in inscription_templates {
                        let offset_occupied = !inscription_outpoint_to_offsets
                            .entry(inscription_template.location.outpoint)
                            .or_default()
                            .insert(inscription_template.location.offset); // return false if item already exist

                        // skip inscription which was created into occupied offset
                        if !inscription_template.leaked && offset_occupied {
                            continue;
                        }

                        // handle token deploy|mint|transfer creation
                        token_cache.parse_token_action(&inscription_template, height, created);
                    }
                }
            }
        }

        data_to_write.push(Box::new(BlockInscriptionPartialsWriter {
            to_remove: partials_to_remove,
            to_write: outpoint_to_partials.into_iter().collect(),
        }));

        data_to_write.push(Box::new(BlockInscriptionOffsetWriter {
            to_remove: inscription_outpoint_to_offsets
                .iter()
                .filter(|(_, offsets)| offsets.is_empty())
                .map(|(outpoint, _)| *outpoint)
                .collect(),
            to_write: inscription_outpoint_to_offsets
                .into_iter()
                .filter(|(_, offsets)| !offsets.is_empty())
                .collect(),
        }));
    }

    fn handle_block(
        data_to_write: &mut Vec<Box<dyn ProcessedData>>,
        block_events: &mut Vec<ServerEvent>,
        history: &mut Vec<(AddressTokenId, HistoryValue)>,
        block_height: u32,
        block: &bellscoin::Block,
        server: &Server,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let current_hash = block.block_hash();

        if reorg_cache.is_some() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
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

        let outpoint_fullhash_to_address: HashMap<_, _> = block
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
            .collect();

        data_to_write.push(Box::new(BlockInfoWriter {
            block_number: block_height,
            block_info,
        }));

        data_to_write.push(Box::new(BlockPrevoutsWriter {
            to_write: block
                .txdata
                .iter()
                .flat_map(|tx| {
                    let txid = tx.txid();
                    tx.output
                        .iter()
                        .enumerate()
                        .map(move |(input_index, txout)| {
                            (
                                OutPoint {
                                    txid,
                                    vout: input_index as u32,
                                },
                                txout.clone(),
                            )
                        })
                })
                .filter(|(_, txout)| !txout.script_pubkey.is_provably_unspendable())
                .collect(),
            to_remove: vec![],
        }));

        data_to_write.push(Box::new(BlockFullHashWriter {
            addresses: outpoint_fullhash_to_address
                .iter()
                .map(|(fullhash, address)| (*fullhash, address.clone()))
                .collect(),
        }));

        if block_height < *START_HEIGHT {
            return Ok(());
        }

        if block.txdata.len() == 1 {
            let new_proof = Server::generate_history_hash(prev_block_proof, &[], &HashMap::new())?;

            data_to_write.push(Box::new(BlockProofWriter {
                block_number: block_height,
                block_proof: new_proof,
            }));

            block_events.push(ServerEvent::NewBlock(
                block_height,
                new_proof,
                block.block_hash(),
            ));

            return Ok(());
        }

        let prevouts = utils::load_prevouts_for_block(server.db.clone(), &block.txdata)?;

        if let Some(cache) = reorg_cache.as_ref() {
            prevouts.iter().for_each(|(key, value)| {
                cache.lock().removed_prevout(*key, value.clone());
            });
        }

        // init token cache
        let (mut token_cache, transfers_to_remove) = {
            let mut token_cache = TokenCache::default();
            let transfers_to_remove: HashSet<_> = prevouts
                .iter()
                .map(|(k, v)| AddressLocation {
                    address: v.script_pubkey.compute_script_hash(),
                    location: Location {
                        outpoint: *k,
                        offset: 0,
                    },
                })
                .collect();

            token_cache
                .valid_transfers
                .extend(server.db.load_transfers(transfers_to_remove.clone()));

            token_cache.all_transfers = token_cache
                .valid_transfers
                .iter()
                .map(|(location, (_, proto))| (*location, proto.clone()))
                .collect();

            (token_cache, transfers_to_remove)
        };

        Self::parse_block(
            server,
            data_to_write,
            block_height,
            created,
            &block.txdata,
            &prevouts,
            &mut token_cache,
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

        data_to_write.push(Box::new(BlockProofWriter {
            block_number: block_height,
            block_proof: new_proof,
        }));

        data_to_write.push(Box::new(BlockHistoryWriter {
            block_number: block_height,
            last_history_id,
            history: history.clone(),
        }));

        if let Some(reorg_cache) = reorg_cache.as_ref() {
            let mut cache = reorg_cache.lock();
            history
                .iter()
                .for_each(|(k, _)| cache.added_history(k.clone()));
        };

        data_to_write.push(Box::new(BlockTokensWriter {
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
        }));

        block_events.push(ServerEvent::NewBlock(
            block_height,
            new_proof,
            block.block_hash(),
        ));

        Ok(())
    }

    pub async fn handle(
        block_height: u32,
        block: bellscoin::Block,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let mut data_to_write: Vec<Box<dyn ProcessedData>> = vec![];
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

    fn convert_to_template(
        payload: &ParseInscription,
        inscription: Inscription,
    ) -> InscriptionTemplate {
        let genesis = {
            InscriptionId {
                txid: payload.partials.genesis_txid,
                index: 0,
            }
        };

        let content_type = inscription.content_type().map(|x| x.to_owned());

        let pointer = inscription.pointer();

        let mut inscription_template = InscriptionTemplate {
            content: inscription.into_body(),
            content_type,
            genesis,
            location: Location {
                offset: 0,
                outpoint: OutPoint {
                    txid: payload.tx.txid(),
                    vout: payload.input_index,
                },
            },
            owner: FullHash::ZERO,
            value: 0,
            leaked: false,
        };

        let Ok((mut vout, mut offset)) = InscriptionSearcher::get_output_index_by_input(
            payload
                .inputs_cum
                .get(payload.input_index as usize)
                .copied(),
            &payload.tx.output,
        ) else {
            inscription_template.leaked = true;
            return inscription_template;
        };

        if let Ok((new_vout, new_offset)) =
            InscriptionSearcher::get_output_index_by_input(pointer, &payload.tx.output)
        {
            vout = new_vout;
            offset = new_offset;
        }

        let location: Location = Location {
            outpoint: OutPoint {
                txid: payload.tx.txid(),
                vout,
            },
            offset,
        };

        let tx_out = &payload.tx.output[vout as usize];

        if tx_out.script_pubkey.is_op_return() {
            inscription_template.owner = *OP_RETURN_HASH;
        } else {
            inscription_template.owner = tx_out.script_pubkey.compute_script_hash();
        }

        inscription_template.location = location;
        inscription_template.value = tx_out.value;

        inscription_template
    }

    fn parse_inscription(payload: ParseInscription) -> ParsedInscriptionResult {
        let parsed = Inscription::from_parts(&payload.partials.parts, payload.input_index);

        match parsed {
            ParsedInscription::None => ParsedInscriptionResult::None,
            ParsedInscription::Partial => ParsedInscriptionResult::Partials,
            ParsedInscription::Single(inscription) => {
                ParsedInscriptionResult::Single(Self::convert_to_template(&payload, inscription))
            }
            ParsedInscription::Many(inscriptions) => ParsedInscriptionResult::Many(
                inscriptions
                    .into_iter()
                    .map(|inscription| Self::convert_to_template(&payload, inscription))
                    .collect(),
            ),
        }
    }
}

pub enum ParsedInscriptionResult {
    None,
    Partials,
    Single(InscriptionTemplate),
    Many(Vec<InscriptionTemplate>),
}

trait ProcessedData: Send + Sync {
    fn write(&self, db: &DB);
}

struct BlockInfoWriter {
    pub block_number: u32,
    pub block_info: BlockInfo,
}

impl ProcessedData for BlockInfoWriter {
    fn write(&self, db: &DB) {
        db.last_block.set((), self.block_number);
        db.block_info
            .set(self.block_number, self.block_info.clone());
    }
}

struct BlockPrevoutsWriter {
    pub to_remove: Vec<OutPoint>,
    pub to_write: Vec<(OutPoint, TxOut)>,
}

impl ProcessedData for BlockPrevoutsWriter {
    fn write(&self, db: &DB) {
        db.prevouts.remove_batch(self.to_remove.clone().into_iter());
        db.prevouts.extend(self.to_write.clone());
    }
}

struct BlockFullHashWriter {
    pub addresses: Vec<(FullHash, String)>,
}

impl ProcessedData for BlockFullHashWriter {
    fn write(&self, db: &DB) {
        db.fullhash_to_address.extend(self.addresses.clone());
    }
}

struct BlockProofWriter {
    pub block_number: u32,
    pub block_proof: sha256::Hash,
}

impl ProcessedData for BlockProofWriter {
    fn write(&self, db: &DB) {
        db.proof_of_history.set(self.block_number, self.block_proof);
    }
}

struct BlockHistoryWriter {
    pub block_number: u32,
    pub last_history_id: u64,
    pub history: Vec<(AddressTokenId, HistoryValue)>,
}

impl ProcessedData for BlockHistoryWriter {
    fn write(&self, db: &DB) {
        let block_events: Vec<_> = self
            .history
            .iter()
            .map(|(address_token_id, _)| address_token_id.clone())
            .sorted_unstable_by_key(|address_token_id| address_token_id.id)
            .collect();

        let outpoint_to_event = self
            .history
            .iter()
            .map(|(address_token_id, history_value)| {
                (history_value.action.outpoint(), address_token_id.clone())
            });

        db.block_events.set(self.block_number, block_events);
        db.outpoint_to_event.extend(outpoint_to_event);
        db.address_token_to_history.extend(self.history.clone());
        db.last_history_id.set((), self.last_history_id);
    }
}

struct BlockTokensWriter {
    pub metas: Vec<(LowerCaseTokenTick, TokenMetaDB)>,
    pub balances: Vec<(AddressToken, TokenBalance)>,
    pub transfers_to_write: Vec<(AddressLocation, TransferProtoDB)>,
    pub transfers_to_remove: Vec<AddressLocation>,
}

impl ProcessedData for BlockTokensWriter {
    fn write(&self, db: &DB) {
        db.token_to_meta.extend(self.metas.clone());
        db.address_token_to_balance.extend(self.balances.clone());
        db.address_location_to_transfer
            .remove_batch(self.transfers_to_remove.clone().into_iter());
        db.address_location_to_transfer
            .extend(self.transfers_to_write.clone());
    }
}

struct BlockInscriptionPartialsWriter {
    pub to_remove: Vec<OutPoint>,
    pub to_write: Vec<(OutPoint, Partials)>,
}

impl ProcessedData for BlockInscriptionPartialsWriter {
    fn write(&self, db: &DB) {
        db.outpoint_to_partials
            .remove_batch(self.to_remove.clone().into_iter());
        db.outpoint_to_partials.extend(self.to_write.clone());
    }
}

struct BlockInscriptionOffsetWriter {
    pub to_remove: Vec<OutPoint>,
    pub to_write: Vec<(OutPoint, HashSet<u64>)>,
}

impl ProcessedData for BlockInscriptionOffsetWriter {
    fn write(&self, db: &DB) {
        db.outpoint_to_inscription_offsets
            .remove_batch(self.to_remove.clone().into_iter());
        db.outpoint_to_inscription_offsets
            .extend(self.to_write.clone());
    }
}
