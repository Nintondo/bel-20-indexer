use bellscoin::ScriptBuf;
use bitcoin_hashes::sha256;
use nint_blk::{
    proto::{tx::EvaluatedTx, Hashed},
    Bellscoin, Coin, CoinType,
};

use crate::inscriptions::{
    indexer::ParsedInscriptionResult,
    leaked::{LeakedInscription, LeakedInscriptions},
    process_data::ProcessedData,
    searcher::InscriptionSearcher,
    structs::{InscriptionMeta, ParsedInscription, Part},
};

use super::*;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curse {
    DuplicateField,
    IncompleteField,
    NotAtOffsetZero,
    NotInFirstInput,
    Pointer,
    Pushnum,
    Reinscription,
    Stutter,
    UnrecognizedEvenField,
}

pub struct ParseInscription<'a> {
    tx: &'a Hashed<EvaluatedTx>,
    input_index: u32,
    inputs_cum: &'a [u64],
    partials: &'a Partials,
    prevouts: &'a HashMap<OutPoint, TxPrevout>,
    coin: CoinType,
}

pub struct Parser<'a> {
    pub server: &'a Server,
    pub token_cache: &'a mut TokenCache,
}

impl Parser<'_> {
    pub fn parse_block(&mut self, height: u32, block: nint_blk::proto::block::Block, prevouts: &HashMap<OutPoint, TxPrevout>, data_to_write: &mut Vec<ProcessedData>) {
        let coin = self.server.indexer.coin;
        let jubilant = height as usize >= coin.jubilee_height.unwrap_or_default();
        let is_p2tr_only = coin.only_p2tr;

        // Hold inscription's partials from db and new in the block
        let mut outpoint_to_partials = Self::load_partials(self.server, prevouts.keys().cloned().collect());

        // Hold inscription's partials to remove from db
        let partials_to_remove: Vec<_> = outpoint_to_partials.iter().map(|x| (*x.0, x.1.clone())).collect();

        let mut inscription_outpoint_to_offsets = Self::load_inscription_outpoint_to_offsets(self.server, prevouts.keys().cloned().collect());

        let prev_offsets = inscription_outpoint_to_offsets.iter().map(|(k, v)| (*k, v.clone())).collect_vec();

        let mut leaked: Option<LeakedInscriptions> = None;

        for tx in &block.txs {
            if tx.value.is_coinbase() {
                leaked = Some(LeakedInscriptions::new(Hashed {
                    hash: tx.hash,
                    value: tx.value.clone(),
                }));

                continue;
            }

            leaked.as_mut().unwrap().add_tx_fee(tx, prevouts);

            let mut inscription_index_in_tx = 0;
            let txid: Txid = tx.hash.into();

            // Optional ad-hoc debug: set DEBUG_TXS=txid1,txid2 to trace reinscription/token decisions.
            let debug_txids: HashSet<Txid> = std::env::var("DEBUG_TXS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .filter_map(|t| Txid::from_str(t.trim()).ok())
                        .collect()
                })
                .unwrap_or_default();
            let log_this_tx = debug_txids.contains(&txid);

            let inputs_cum = InscriptionSearcher::calc_offsets(tx, prevouts).expect("failed to find all txos to calculate offsets");

            // Additionally compute pre-fee cumulative input offsets (ord's total_input_value).
            // inputs_cum is used for routing (post-fee), while inputs_cum_prefee mirrors
            // ord's notion of cumulative input value before subtracting any fees.
            let mut inputs_cum_prefee: Vec<u64> = Vec::with_capacity(tx.value.inputs.len());
            let mut acc_prefee: u64 = 0;
            for txin in &tx.value.inputs {
                inputs_cum_prefee.push(acc_prefee);
                acc_prefee = acc_prefee
                    .saturating_add(prevouts.get(&txin.outpoint).map(|pv| pv.value).unwrap_or(0));
            }

            // For ord-style reinscription detection (p2tr-only coins), track how many
            // inscriptions we have already seen at each *input offset* in this
            // transaction. This mirrors ord's `inscribed_offsets` map which is keyed
            // by the running input value (before applying any pointer).
            //
            // Key:   global input offset (like ord's `offset = total_input_value`)
            // Value: (initial_cursed_or_vindicated, count_in_this_tx)
            //
            // - `initial_cursed_or_vindicated` is approximated as the `base_cursed`
            //   value of the first inscription we see at this offset in this tx (or
            //   the first old inscription seeded for this offset).
            // - `count` tracks how many inscriptions have been attached to this offset
            //   in this tx so that we can reproduce ord's reinscription rules:
            //     * if count > 1       => Reinscription
            //     * if count == 1 and
            //       initial was not
            //       cursed/vindicated  => Reinscription
            let mut inscribed_offsets: BTreeMap<u64, (bool, u8)> = BTreeMap::new();

            for (input_index, txin) in tx.value.inputs.iter().enumerate() {
                // handle inscription moves
                if let Some(inscription_offsets) = inscription_outpoint_to_offsets.remove(&txin.outpoint) {
                    for (inscription_offset, initial_cursed) in inscription_offsets {
                        let old_location = Location {
                            outpoint: txin.outpoint,
                            offset: inscription_offset,
                        };

                        let is_token_transfer_move = self.token_cache.all_transfers.contains_key(&old_location);

                        // Global input-space offset for this old inscription. This is
                        // our analogue of ord's `offset = total_input_value +
                        // old_satpoint_offset` when seeding `inscribed_offsets` with
                        // inscriptions that arrive on transaction inputs.
                        // Use pre-fee input offset for seeding ord-style map
                        let offset = inputs_cum_prefee.get(input_index).map(|x| *x + inscription_offset);

                        // Seed ord-style `inscribed_offsets` for p2tr-only coins so
                        // that reinscription detection later can see that this input
                        // offset already carried inscriptions before new ones are
                        // created in this transaction.
                        if is_p2tr_only {
                            if let Some(global_offset) = offset {
                                let entry = inscribed_offsets
                                    .entry(global_offset)
                                    // `initial_cursed` is the base_cursed flag we
                                    // stored for this satpoint when its initial
                                    // inscription was created. We reuse it as our
                                    // "initial cursed or vindicated" indicator for
                                    // this offset.
                                    .or_insert((initial_cursed, 0));
                                entry.1 = entry.1.saturating_add(1);

                                if log_this_tx {
                                    eprintln!(
                                        "[DEBUG_TX] seed-old tx={} input={} global_offset={} initial_cursed={} count={}",
                                        txid, input_index, global_offset, initial_cursed, entry.1
                                    );
                                }
                            }
                        }

                        match InscriptionSearcher::get_output_index_by_input(offset, &tx.value.outputs) {
                            Ok((new_vout, new_offset)) => {
                                let new_outpoint = OutPoint { txid, vout: new_vout };

                                inscription_outpoint_to_offsets
                                    .entry(new_outpoint)
                                    .or_default()
                                    .entry(new_offset)
                                    .or_insert(initial_cursed);

                                // handle move of token transfer
                                if is_token_transfer_move {
                                    if ScriptBuf::from_bytes(tx.value.outputs[new_vout as usize].out.script_pubkey.clone()).is_op_return() {
                                        self.token_cache.burned_transfer(old_location, txid, new_vout);
                                    } else {
                                        let owner = bellscoin::hashes::sha256::Hash::hash(&tx.value.outputs[new_vout as usize].out.script_pubkey);
                                        self.token_cache.transferred(old_location, owner.into(), txid, new_vout);
                                    };
                                }
                            }
                            Err(_) => {
                                // handle leaked move of token transfer
                                if is_token_transfer_move {
                                    // because of token protocol leaked token amount
                                    // comeback to owner
                                    let recipient = prevouts.get(&txin.outpoint).expect("Owner of token transfer must exist").script_hash;
                                    self.token_cache.transferred(old_location, recipient, txid, 0);
                                }
                                leaked.as_mut().unwrap().add(input_index, tx, inscription_offset, prevouts, LeakedInscription::Move);
                            }
                        }
                    }
                }

                // handle inscription creation
                if jubilant || input_index == 0 {
                    let mut partials = outpoint_to_partials.remove(&txin.outpoint).unwrap_or(Partials {
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
                            script_buffer: txin.script_sig.clone(),
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
                            coin: self.server.indexer.coin,
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
                            if tx.value.outputs.get(input_index).is_some() {
                                outpoint_to_partials.insert(OutPoint { txid, vout: input_index as u32 }, partials);
                            }
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
                                inscription_templates.iter_mut().for_each(|inscription_template| {
                                    inscription_template.genesis.index = inscription_index_in_tx;
                                    inscription_index_in_tx += 1;
                                });
                            }

                            inscription_templates
                        }
                    };

                    for mut inscription_template in inscription_templates {
                        let location = inscription_template.location;
                        let pointer_raw = inscription_template.pointer_value;

                        // Determine whether this satpoint was already occupied before this inscription.
                        let offsets_map = inscription_outpoint_to_offsets.entry(location.outpoint).or_default();
                        let had_previous_at_location = offsets_map.contains_key(&location.offset);

                        // For non-p2tr coins, always track occupancy like legacy HashSet<u64> did.
                        // The bool payload is unused for these coins, so we store `false`.
                        if !is_p2tr_only && !had_previous_at_location {
                            offsets_map.insert(location.offset, false);
                        }

                        // For non-p2tr coins keep legacy reinscription skipping behavior.
                        if !is_p2tr_only {
                            let mut offset_occupied = had_previous_at_location;

                            // This is only for BELLS
                            if coin.name == Bellscoin::NAME {
                                offset_occupied = false;
                            }

                            let is_reinscription_legacy = offset_occupied && (!jubilant || coin.only_p2tr);

                            // skip inscription which was created into occupied offset
                            if !inscription_template.leaked && is_reinscription_legacy {
                                continue;
                            }
                        }

                        if is_p2tr_only {
                            // --- ord-style curse classification for p2tr-only coins ---
                            let mut curse: Option<Curse> = None;

                            if inscription_template.unrecognized_even_field {
                                curse = Some(Curse::UnrecognizedEvenField);
                            } else if inscription_template.duplicate_field {
                                curse = Some(Curse::DuplicateField);
                            } else if inscription_template.incomplete_field {
                                curse = Some(Curse::IncompleteField);
                            } else if inscription_template.input_index != 0 {
                                curse = Some(Curse::NotInFirstInput);
                            } else if inscription_template.envelope_offset != 0 {
                                curse = Some(Curse::NotAtOffsetZero);
                            } else if inscription_template.has_pointer {
                                curse = Some(Curse::Pointer);
                            } else if inscription_template.pushnum {
                                curse = Some(Curse::Pushnum);
                            } else if inscription_template.stutter {
                                curse = Some(Curse::Stutter);
                            } else {
                                // Potential reinscription curse.
                                //
                                // For p2tr-only coins we now follow ord's logic more
                                // closely by keying reinscription detection off the
                                // *input offset* (the running total input value
                                // before this input), not the final satpoint
                                // (outpoint, offset). This matches ord's use of
                                // `inscribed_offsets.get(&offset)` where `offset`
                                // is the `total_input_value` prior to applying any
                                // pointer.
                                //
                                // `inputs_cum[input_index]` is our analogue of
                                // ord's `total_input_value` at the beginning of
                                // this input.
                                // Use pre-fee cumulative offset (ord's total_input_value)
                                if let Some(global_input_offset) = inputs_cum_prefee.get(input_index).copied() {
                                    if let Some((initial_cursed_or_vindicated, count)) =
                                        inscribed_offsets.get(&global_input_offset)
                                    {
                                        if *count > 1 {
                                            // More than one inscription has already
                                            // been attached to this input offset
                                            // in this tx. Any further inscription
                                            // is a reinscription.
                                            curse = Some(Curse::Reinscription);
                                        } else if !*initial_cursed_or_vindicated {
                                            // Exactly one prior inscription at this
                                            // input offset and it was blessed (not
                                            // cursed or vindicated). The *second*
                                            // one becomes a reinscription.
                                            curse = Some(Curse::Reinscription);
                                        }
                                    }
                                }
                            }

                            // Jubilee + unbound logic.
                            let input_value = prevouts
                                .get(&tx.value.inputs[input_index].outpoint)
                                .map(|pv| pv.value)
                                .unwrap_or(0);

                            // Base cursed flag used for ord-style reinscription logic.
                            // This mirrors ord's notion of "cursed or vindicated" – it is true
                            // whenever a curse pattern is present, regardless of jubilee.
                            let base_cursed = curse.is_some();

                            // BRC-20-specific cursed flag:
                            //  - For Litecoin (ltc-20), only pre-jubilee curses should be treated
                            //    as BRC-20-invalid (post-jubilee they are "vindicated").
                            //  - For all other coins (including Bitcoin), preserve existing
                            //    behaviour and treat any curse as BRC-20-invalid.
                            let cursed_for_brc20 = if coin.brc_name == "ltc-20" {
                                base_cursed && !jubilant
                            } else {
                                base_cursed
                            };

                            let vindicated = base_cursed && jubilant;
                            let unbound = input_value == 0
                                || matches!(curse, Some(Curse::UnrecognizedEvenField))
                                || inscription_template.unrecognized_even_field;

                            // Track reinscriptions per input offset for p2tr-only coins.
                            //
                            // This is the in-memory analogue of ord's
                            // `inscribed_offsets.entry(offset)` where `offset` is
                            // `total_input_value` before examining this input.
                            // Record this inscription under pre-fee input offset as in ord
                            if let Some(global_input_offset) = inputs_cum_prefee.get(input_index).copied() {
                                let offset_to_track = inscription_template.pointer_value.unwrap_or(global_input_offset);

                                let entry = inscribed_offsets
                                    .entry(offset_to_track)
                                    // If this is the first inscription we see at
                                    // this offset in this tx, the "initial cursed
                                    // or vindicated" flag is simply this
                                    // inscription's base_cursed value. Later
                                    // inscriptions at the same offset keep this
                                    // initial flag unchanged.
                                    .or_insert((base_cursed, 0));

                                // Saturating add to avoid any possible overflow,
                                // though realistically we never expect more than a
                                // handful of inscriptions per input.
                                entry.1 = entry.1.saturating_add(1);

                                if log_this_tx {
                                    eprintln!(
                                        concat!(
                                            "[DEBUG_TX] new-inscr tx={} input={} base={} ptr={:?} ",
                                            "curse={:?} base_cursed={} initial_flag={} count={} owner_opret={} location_offset={}"
                                        ),
                                        txid,
                                        input_index,
                                        global_input_offset,
                                        pointer_raw,
                                        curse,
                                        base_cursed,
                                        entry.0,
                                        entry.1,
                                        inscription_template.owner == *OP_RETURN_HASH,
                                        location.offset,
                                    );
                                }
                            }

                            // Persist initial cursed/vindicated state for this location if it's the first inscription here.
                            if !had_previous_at_location {
                                offsets_map.insert(location.offset, base_cursed);
                            }

                            inscription_template.cursed_for_brc20 = cursed_for_brc20;
                            inscription_template.unbound = unbound;
                            inscription_template.reinscription =
                                matches!(curse, Some(Curse::Reinscription));
                            inscription_template.vindicated = vindicated;
                        }

                        // handle token deploy|mint|transfer creation
                        self.token_cache.parse_token_action(&inscription_template, height, block.header.value.timestamp);
                    }
                }
            }
        }

        leaked.unwrap().get_leaked_inscriptions().for_each(|location| {
            inscription_outpoint_to_offsets
                .entry(location.outpoint)
                .or_default()
                .entry(location.offset)
                .or_insert(false);
        });

        data_to_write.push(ProcessedData::InscriptionPartials {
            to_remove: partials_to_remove,
            to_write: outpoint_to_partials.into_iter().collect(),
        });

        data_to_write.push(ProcessedData::InscriptionOffset {
            to_remove: prev_offsets,
            to_write: inscription_outpoint_to_offsets.into_iter().collect(),
        });
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

    fn load_inscription_outpoint_to_offsets(server: &Server, outpoints: Vec<OutPoint>) -> HashMap<OutPoint, BTreeMap<u64, bool>> {
        server
            .db
            .outpoint_to_inscription_offsets
            .multi_get_kv(outpoints.iter(), false)
            .into_iter()
            .map(|(k, v)| (*k, v))
            .collect()
    }

    fn parse_inscription(payload: ParseInscription, leaked: &mut LeakedInscriptions) -> ParsedInscriptionResult {
        let parsed = Inscription::from_parts(&payload.partials.parts, payload.input_index, payload.coin);

        match parsed {
            ParsedInscription::None => ParsedInscriptionResult::None,
            ParsedInscription::Partial => ParsedInscriptionResult::Partials,
            ParsedInscription::Single(inscription, meta) => Self::convert_to_template(&payload, inscription, meta, leaked)
                .map(ParsedInscriptionResult::Single)
                .unwrap_or(ParsedInscriptionResult::None),
            ParsedInscription::Many(inscriptions) => ParsedInscriptionResult::Many(
                inscriptions
                    .into_iter()
                    .filter_map(|(inscription, meta)| Self::convert_to_template(&payload, inscription, meta, leaked))
                    .collect(),
            ),
        }
    }

    fn convert_to_template(payload: &ParseInscription, inscription: Inscription, meta: InscriptionMeta, leaked: &mut LeakedInscriptions) -> Option<InscriptionTemplate> {
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
                    txid: payload.tx.hash.into(),
                    vout: payload.input_index,
                },
            },
            owner: FullHash::ZERO,
            value: 0,
            leaked: false,
             // ord/OPI compatibility fields, filled from meta and inscription
             input_index: meta.input_index,
             envelope_offset: meta.envelope_offset,
             duplicate_field: meta.duplicate_field,
             incomplete_field: meta.incomplete_field,
             unrecognized_even_field: meta.unrecognized_even_field,
             has_pointer: meta.has_pointer,
             pushnum: meta.pushnum,
             stutter: meta.stutter,
             cursed_for_brc20: false,
             unbound: false,
             reinscription: false,
             vindicated: false,
             pointer_value: None,
        };

        let Ok((mut vout, mut offset)) = InscriptionSearcher::get_output_index_by_input(payload.inputs_cum.get(payload.input_index as usize).copied(), &payload.tx.value.outputs)
        else {
            leaked.add(payload.input_index as usize, payload.tx, 0, payload.prevouts, LeakedInscription::Creation);
            return None;
        };

        if let Ok((new_vout, new_offset)) = InscriptionSearcher::get_output_index_by_input(pointer, &payload.tx.value.outputs) {
            vout = new_vout;
            offset = new_offset;
            inscription_template.pointer_value = pointer;
        }

        let location: Location = Location {
            outpoint: OutPoint {
                txid: payload.tx.hash.into(),
                vout,
            },
            offset,
        };

        let tx_out = &payload.tx.value.outputs[vout as usize];

        if ScriptBuf::from_bytes(tx_out.out.script_pubkey.clone()).is_op_return() {
            inscription_template.owner = *OP_RETURN_HASH;
        } else {
            inscription_template.owner = sha256::Hash::hash(&tx_out.out.script_pubkey).into();
        }

        inscription_template.location = location;
        inscription_template.value = tx_out.out.value;

        Some(inscription_template)
    }
}
