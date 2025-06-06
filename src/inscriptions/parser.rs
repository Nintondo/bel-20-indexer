use crate::inscriptions::indexer::ParsedInscriptionResult;
use crate::inscriptions::leaked::{LeakedInscription, LeakedInscriptions};
use crate::inscriptions::processe_data::ProcessedData;
use crate::inscriptions::searcher::InscriptionSearcher;
use crate::inscriptions::structs::{Inscription, ParsedInscription, Part, Partials};
use crate::inscriptions::Location;
use crate::server::Server;
use crate::tokens::{ComputeScriptHash, FullHash, InscriptionId, InscriptionTemplate, TokenCache};
use crate::{JUBILEE_HEIGHT, OP_RETURN_HASH};
use bellscoin::{OutPoint, Transaction, TxOut};
use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct ParseInscription<'a> {
    tx: &'a Transaction,
    input_index: u32,
    inputs_cum: &'a [u64],
    partials: &'a Partials,
    prevouts: &'a HashMap<OutPoint, TxOut>,
}

pub struct Parser<'a> {
    pub server: &'a Server,
    pub reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    pub token_cache: &'a mut TokenCache,
}

impl Parser<'_> {
    pub fn parse_block(
        &mut self,
        height: u32,
        block: &bellscoin::Block,
        prevouts: &HashMap<OutPoint, TxOut>,
        data_to_write: &mut Vec<ProcessedData>,
    ) {
        let is_jubilee_height = height as usize >= *JUBILEE_HEIGHT;

        // Hold inscription's partials from db and new in the block
        let mut outpoint_to_partials =
            Self::load_partials(self.server, prevouts.keys().cloned().collect());

        // Hold inscription's partials to remove from db
        let partials_to_remove: Vec<_> = outpoint_to_partials.keys().copied().collect();

        let mut inscription_outpoint_to_offsets = Self::load_inscription_outpoint_to_offsets(
            self.server,
            prevouts.keys().cloned().collect(),
        );

        let prev_offsets = inscription_outpoint_to_offsets
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect_vec();

        let mut leaked: Option<LeakedInscriptions> = None;

        for tx in &block.txdata {
            if tx.is_coin_base() {
                leaked = Some(LeakedInscriptions::new(tx.clone()));

                continue;
            }

            leaked.as_mut().unwrap().add_tx_fee(tx, prevouts);

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
                            self.token_cache.all_transfers.contains_key(&old_location);

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
                                        self.token_cache.burned_transfer(
                                            old_location,
                                            txid,
                                            new_vout,
                                        );
                                    } else {
                                        let owner = tx.output[new_vout as usize]
                                            .script_pubkey
                                            .compute_script_hash();
                                        self.token_cache.transferred(
                                            old_location,
                                            owner,
                                            txid,
                                            new_vout,
                                        );
                                    };
                                }
                            }
                            Err(_) => {
                                // handle leaked move of token transfer
                                if is_token_transfer_move {
                                    // because of token protocol leaked token amount
                                    // comeback to owner
                                    let recipient = prevouts
                                        .get(&txin.previous_output)
                                        .expect("Owner of token transfer must exist")
                                        .script_pubkey
                                        .compute_script_hash();
                                    self.token_cache
                                        .transferred(old_location, recipient, txid, 0);
                                }
                                leaked.as_mut().unwrap().add(
                                    input_index,
                                    tx,
                                    offset.unwrap_or_default(),
                                    prevouts,
                                    LeakedInscription::Move,
                                );
                            }
                        }
                    }
                }

                // handle inscription creation
                if is_jubilee_height || input_index == 0 {
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

                    let parsed_result = Self::parse_inscription(
                        ParseInscription {
                            tx,
                            input_index: input_index as u32,
                            inputs_cum: &inputs_cum,
                            partials: &partials,
                            prevouts,
                        },
                        leaked.as_mut().unwrap(),
                    );

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
                        let mut offset_occupied = !inscription_outpoint_to_offsets
                            .entry(inscription_template.location.outpoint)
                            .or_default()
                            .insert(inscription_template.location.offset); // return false if item already exist

                        // This is only for BELLS
                        if *JUBILEE_HEIGHT == 133_000 {
                            offset_occupied = false;
                        }

                        // skip inscription which was created into occupied offset
                        if !inscription_template.leaked && offset_occupied && !is_jubilee_height {
                            continue;
                        }

                        // handle token deploy|mint|transfer creation
                        self.token_cache.parse_token_action(
                            &inscription_template,
                            height,
                            block.header.time,
                        );
                    }
                }
            }
        }

        leaked
            .unwrap()
            .get_leaked_inscriptions()
            .for_each(|location| {
                inscription_outpoint_to_offsets
                    .entry(location.outpoint)
                    .or_default()
                    .insert(location.offset);
            });

        data_to_write.push(ProcessedData::InscriptionPartials {
            to_remove: partials_to_remove,
            to_write: outpoint_to_partials.into_iter().collect(),
        });

        {
            if let Some(reorg_cache) = self.reorg_cache.as_ref() {
                let mut cache = reorg_cache.lock();
                cache.add_inscription_offsets(
                    prev_offsets.clone(),
                    inscription_outpoint_to_offsets.keys().copied().collect(),
                );
            };

            data_to_write.push(ProcessedData::InscriptionOffset {
                to_remove: prev_offsets.iter().map(|x| x.0).collect(),
                to_write: inscription_outpoint_to_offsets.into_iter().collect(),
            });
        }
    }

    fn load_partials(server: &Server, outpoints: Vec<OutPoint>) -> HashMap<OutPoint, Partials> {
        server
            .db
            .outpoint_to_partials
            .multi_get_kv(outpoints.iter(), false)
            .into_iter()
            .map(|(k, v)| (*k, v))
            .collect()
    }

    fn load_inscription_outpoint_to_offsets(
        server: &Server,
        outpoints: Vec<OutPoint>,
    ) -> HashMap<OutPoint, HashSet<u64>> {
        server
            .db
            .outpoint_to_inscription_offsets
            .multi_get_kv(outpoints.iter(), false)
            .into_iter()
            .map(|(k, v)| (*k, v))
            .collect()
    }

    fn parse_inscription(
        payload: ParseInscription,
        leaked: &mut LeakedInscriptions,
    ) -> ParsedInscriptionResult {
        let parsed = Inscription::from_parts(&payload.partials.parts, payload.input_index);

        match parsed {
            ParsedInscription::None => ParsedInscriptionResult::None,
            ParsedInscription::Partial => ParsedInscriptionResult::Partials,
            ParsedInscription::Single(inscription) => {
                Self::convert_to_template(&payload, inscription, leaked)
                    .map(ParsedInscriptionResult::Single)
                    .unwrap_or(ParsedInscriptionResult::None)
            }
            ParsedInscription::Many(inscriptions) => ParsedInscriptionResult::Many(
                inscriptions
                    .into_iter()
                    .filter_map(|inscription| {
                        Self::convert_to_template(&payload, inscription, leaked)
                    })
                    .collect(),
            ),
        }
    }

    fn convert_to_template(
        payload: &ParseInscription,
        inscription: Inscription,
        leaked: &mut LeakedInscriptions,
    ) -> Option<InscriptionTemplate> {
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
            leaked.add(
                payload.input_index as usize,
                payload.tx,
                0,
                payload.prevouts,
                LeakedInscription::Creation,
            );
            return None;
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

        Some(inscription_template)
    }
}
