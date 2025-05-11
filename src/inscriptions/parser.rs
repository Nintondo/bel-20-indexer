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

        let mut transfers = vec![];

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
        let partials_to_remove: HashSet<_> = outpoint_to_partials.keys().cloned().collect();

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
                transfers.extend(
                    token_cache
                        .valid_transfers
                        .range(
                            Location {
                                outpoint: txin.previous_output,
                                offset: 0,
                            }..=Location {
                                outpoint: txin.previous_output,
                                offset: u64::MAX,
                            },
                        )
                        .map(|(k, (address, proto))| (*k, (*address, proto.clone()))),
                );

                // handle inscription moves
                if let Some(inscription_offsets) =
                    inscription_outpoint_to_offsets.remove(&txin.previous_output)
                {
                    for inscription_offset in inscription_offsets {
                        let offset = inputs_cum.get(input_index).map(|x| *x + inscription_offset);
                        match InscriptionSearcher::get_output_index_by_input(offset, &tx.output) {
                            Ok((new_vout, new_offset)) => {
                                inscription_outpoint_to_offsets
                                    .entry(OutPoint {
                                        txid,
                                        vout: new_vout,
                                    })
                                    .or_default()
                                    .insert(new_offset);
                            }
                            _ => todo!(), // TODO handle leak
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
                        if let Some(proto) =
                            token_cache.parse_token_action(&inscription_template, height, created)
                        {
                            transfers.push((
                                inscription_template.location,
                                (
                                    inscription_template.owner,
                                    TransferProtoDB::from_proto(proto, height),
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    pub async fn handle(
        block_height: u32,
        block: bellscoin::Block,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let current_hash = block.block_hash();
        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
            cache.lock().new_block(block_height, last_history_id);
        }

        let block_info = BlockInfo {
            created: block.header.time,
            hash: current_hash,
        };

        server.db.block_info.set(block_height, block_info);

        if reorg_cache.is_some() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let created = block.header.time;

        let prev_block_height = block_height.checked_sub(1).unwrap_or_default();
        let prev_block_proof = server
            .db
            .proof_of_history
            .get(prev_block_height)
            .unwrap_or(*DEFAULT_HASH);

        server.db.fullhash_to_address.extend(
            block.txdata.iter().flat_map(|x| &x.output).filter_map(|x| {
                let fullhash = x.script_pubkey.compute_script_hash();
                let payload = bellscoin::address::Payload::from_script(&x.script_pubkey);
                if x.script_pubkey.is_op_return() || payload.is_err() {
                    return None;
                }
                Some((fullhash, server.address_decoder.encode(&payload.unwrap())))
            }),
        );

        let prevouts = block
            .txdata
            .iter()
            .flat_map(|x| {
                let txid = x.txid();
                x.output.iter().enumerate().map(move |(idx, vout)| {
                    (
                        OutPoint {
                            txid,
                            vout: idx as u32,
                        },
                        vout,
                    )
                })
            })
            .filter(|x| !x.1.script_pubkey.is_provably_unspendable());

        server.db.prevouts.extend(prevouts);

        if block_height < *START_HEIGHT {
            server.db.last_block.set((), block_height);
            return Ok(());
        }

        if block.txdata.len() == 1 {
            server.db.last_block.set((), block_height);
            let new_proof = Server::generate_history_hash(prev_block_proof, &[], &HashMap::new())?;
            server.db.proof_of_history.set(block_height, new_proof);
            server
                .event_sender
                .send(ServerEvent::NewBlock(
                    block_height,
                    new_proof,
                    block.block_hash(),
                ))
                .ok();
            return Ok(());
        }

        let mut token_cache = TokenCache::default();
        let prevouts = utils::load_prevouts_for_block(server.db.clone(), &block.txdata)?;

        if let Some(cache) = reorg_cache.as_ref() {
            prevouts.iter().for_each(|(key, value)| {
                cache.lock().removed_prevout(*key, value.clone());
            });
        }

        token_cache.valid_transfers.extend(
            server.db.load_transfers(
                prevouts
                    .iter()
                    .map(|(k, v)| AddressLocation {
                        address: v.script_pubkey.compute_script_hash(),
                        location: Location {
                            outpoint: *k,
                            offset: 0,
                        },
                    })
                    .collect(),
            ),
        );

        Self::parse_block(
            &server,
            block_height,
            created,
            &block.txdata,
            &prevouts,
            &mut token_cache,
        );

        token_cache.load_tokens_data(&server.db)?;

        let mut fullhash_to_load = HashSet::new();

        let history = token_cache
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
            .collect_vec();

        let rest_addresses = server
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

        let new_proof = Server::generate_history_hash(prev_block_proof, &history, &rest_addresses)?;
        server.db.proof_of_history.set(block_height, new_proof);

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

        server.db.address_token_to_history.extend(history.clone());

        server.db.token_to_meta.extend(
            token_cache
                .tokens
                .into_iter()
                .map(|(k, v)| (k, TokenMetaDB::from(v))),
        );

        server
            .db
            .address_token_to_balance
            .extend(token_cache.token_accounts);

        server.db.address_location_to_transfer.extend(
            token_cache
                .valid_transfers
                .into_iter()
                .map(|(location, (address, proto))| (AddressLocation { address, location }, proto)),
        );

        server.db.last_block.set((), block_height);
        server.db.last_history_id.set((), last_history_id);

        *server.last_indexed_address_height.write().await = block_height;

        server
            .event_sender
            .send(ServerEvent::NewBlock(
                block_height,
                new_proof,
                block.block_hash(),
            ))
            .ok();

        match server.raw_event_sender.send(history) {
            Ok(_) => {}
            _ => {
                if !server.token.is_cancelled() {
                    panic!("Failed to send raw event");
                }
            }
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
