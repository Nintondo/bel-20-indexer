use bellscoin::ScriptBuf;
use bitcoin_hashes::sha256;
use nint_blk::{
    proto::{tx::EvaluatedTx, Hashed},
    Bellscoin, Coin, CoinType,
};

use crate::db::OccupancyState;
use crate::inscriptions::{
    indexer::ParsedInscriptionResult,
    leaked::{LeakedInscription, LeakedInscriptions},
    process_data::ProcessedData,
    searcher::InscriptionSearcher,
    structs::{InscriptionMeta, ParsedInscription, Part},
};
use crate::tokens::InscriptionId;

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

// Helper used for ord-style reinscription detection: an inscription is cursed
// when this input offset has already carried more than one inscription, or the
// first inscription at this offset was not cursed/vindicated.
#[derive(Clone, Debug)]
struct InscribedOffsetState {
    first_inscription: Option<InscriptionId>,
    initial_cursed_or_vindicated: bool,
    count: u8,
}

impl InscribedOffsetState {
    fn from_occupancy(state: &OccupancyState) -> Self {
        Self {
            first_inscription: state.first_inscription,
            initial_cursed_or_vindicated: state.initial_cursed_or_vindicated,
            count: 0,
        }
    }

    fn bump(&mut self, delta: u8) {
        self.count = self.count.saturating_add(delta.max(1));
    }
}

fn is_reinscription(inscribed_offsets: &BTreeMap<u64, InscribedOffsetState>, global_input_offset: u64) -> bool {
    inscribed_offsets
        .get(&global_input_offset)
        .map(|entry| entry.count > 1 || !entry.initial_cursed_or_vindicated)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn spend_with_multiple_existing_inscriptions_curses_reinscription() {
        // UTXO already carried two inscriptions at the same input offset.
        let mut inscribed_offsets: BTreeMap<u64, InscribedOffsetState> = BTreeMap::new();
        inscribed_offsets.insert(
            10,
            InscribedOffsetState {
                first_inscription: None,
                initial_cursed_or_vindicated: true,
                count: 2,
            },
        );

        assert!(is_reinscription(&inscribed_offsets, 10));

        // Any curse should mark BRC-20 invalid for non-LTC coins.
        let coin = CoinType::default(); // Bitcoin settings, only_p2tr = true
        assert!(compute_cursed_for_brc20(coin, true, false));
    }

    #[test]
    fn repeated_inscriptions_in_same_tx_are_cursed_even_with_pointer() {
        let mut inscribed_offsets: BTreeMap<u64, InscribedOffsetState> = BTreeMap::new();
        let global_offset = 0;

        // First inscription at this input offset is allowed.
        assert!(!is_reinscription(&inscribed_offsets, global_offset));
        let (initial_flag_first, count_first) =
            increment_inscription_count(&mut inscribed_offsets, global_offset, false, None);
        assert!(!initial_flag_first);
        assert_eq!(count_first, 1);

        // Second inscription (same input offset, whether pointered or not) is cursed.
        assert!(is_reinscription(&inscribed_offsets, global_offset));
        let (initial_flag_second, count_second) =
            increment_inscription_count(&mut inscribed_offsets, global_offset, true, None);
        assert!(!initial_flag_second);
        assert_eq!(count_second, 2);

        // A pointer-based inscription would still hash to the same input offset key.
        assert!(is_reinscription(&inscribed_offsets, global_offset));
    }

    #[test]
    fn brc20_gating_rejects_reinscriptions() {
        let coin = CoinType::default(); // brc-20
        assert!(compute_cursed_for_brc20(coin, true, false));

        // For ltc-20, curses before jubilee are rejected; after jubilee they are vindicated.
        let ltc_coin = CoinType {
            brc_name: "ltc-20",
            ..CoinType::default()
        };
        assert!(compute_cursed_for_brc20(ltc_coin, true, false));
        assert!(!compute_cursed_for_brc20(ltc_coin, true, true));
    }

    #[test]
    fn cursed_first_inscription_allows_one_reinscription() {
        let mut inscribed_offsets: BTreeMap<u64, InscribedOffsetState> = BTreeMap::new();
        seed_offset_from_state(&mut inscribed_offsets, 0, OccupancyState::from_legacy(true, 1));

        assert!(!is_reinscription(&inscribed_offsets, 0));

        // After recording the second inscription, the next attempt (third overall)
        // becomes a cursed reinscription.
        let _ = increment_inscription_count(&mut inscribed_offsets, 0, true, None);
        assert!(is_reinscription(&inscribed_offsets, 0));
    }

    #[test]
    fn blessed_first_inscription_curses_second() {
        let mut inscribed_offsets: BTreeMap<u64, InscribedOffsetState> = BTreeMap::new();
        seed_offset_from_state(&mut inscribed_offsets, 0, OccupancyState::from_legacy(false, 1));

        assert!(is_reinscription(&inscribed_offsets, 0));
    }
}

// Increment the inscription counter for a given input offset, preserving the
// initial cursed/vindicated flag. Returns the stored tuple after increment.
fn increment_inscription_count(
    inscribed_offsets: &mut BTreeMap<u64, InscribedOffsetState>,
    global_input_offset: u64,
    base_cursed: bool,
    candidate_first: Option<InscriptionId>,
) -> (bool, u8) {
    let entry = inscribed_offsets
        .entry(global_input_offset)
        .or_insert_with(|| InscribedOffsetState {
            first_inscription: candidate_first.clone(),
            initial_cursed_or_vindicated: base_cursed,
            count: 0,
        });

    if entry.first_inscription.is_none() && candidate_first.is_some() {
        entry.first_inscription = candidate_first;
    }

    let initial_flag = entry.initial_cursed_or_vindicated;
    entry.bump(1);
    (initial_flag, entry.count)
}

fn seed_offset_from_state(
    inscribed_offsets: &mut BTreeMap<u64, InscribedOffsetState>,
    global_input_offset: u64,
    state: OccupancyState,
) {
    let entry = inscribed_offsets
        .entry(global_input_offset)
        .or_insert_with(|| InscribedOffsetState::from_occupancy(&state));
    entry.bump(state.count.max(1));
    if entry.first_inscription.is_none() {
        entry.first_inscription = state.first_inscription;
    }
    if entry.count == state.count.max(1) {
        entry.initial_cursed_or_vindicated = state.initial_cursed_or_vindicated;
    }
}

fn compute_cursed_for_brc20(coin: CoinType, base_cursed: bool, jubilant: bool) -> bool {
    if coin.brc_name == "ltc-20" {
        base_cursed && !jubilant
    } else {
        base_cursed
    }
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
            let mut inscribed_offsets: BTreeMap<u64, InscribedOffsetState> = BTreeMap::new();
            let mut per_sat_counts: HashMap<(OutPoint, u64), u32> = HashMap::new();

            for (input_index, txin) in tx.value.inputs.iter().enumerate() {
                // handle inscription moves
                if let Some(inscription_offsets) = inscription_outpoint_to_offsets.remove(&txin.outpoint) {
                    for (inscription_offset, occupancy) in inscription_offsets {
                        let old_location = Location {
                            outpoint: txin.outpoint,
                            offset: inscription_offset,
                        };

                        let is_token_transfer_move = self.token_cache.all_transfers.contains_key(&old_location);

                        // Ord seeds `inscribed_offsets` for old inscriptions at
                        // `offset = total_input_value + old_satpoint_offset`.
                        // Here `input_base_prefee` is that `total_input_value`,
                        // so seeding at `input_base_prefee + inscription_offset`
                        // mirrors ord's offset computation.
                        let input_base_prefee = inputs_cum_prefee.get(input_index).copied();
                        let satpoint_offset = inputs_cum.get(input_index).map(|x| *x + inscription_offset);

                        // Seed ord-style `inscribed_offsets` for p2tr-only coins so
                        // that reinscription detection later can see that this input
                        // offset already carried inscriptions before new ones are
                        // created in this transaction.
                        if is_p2tr_only {
                            if let Some(input_base) = input_base_prefee {
                                let seed_key = input_base.saturating_add(inscription_offset);

                                seed_offset_from_state(
                                    &mut inscribed_offsets,
                                    seed_key,
                                    occupancy.clone(),
                                );

                                if log_this_tx {
                                    if let Some(entry) = inscribed_offsets.get(&seed_key) {
                                        eprintln!(
                                            "[DEBUG_TX] seed-old tx={} input={} global_offset={} initial_cursed={} count={}",
                                            txid,
                                            input_index,
                                            seed_key,
                                            entry.initial_cursed_or_vindicated,
                                            entry.count
                                        );
                                    }
                                }
                            }
                        }

                        match InscriptionSearcher::get_output_index_by_input(satpoint_offset, &tx.value.outputs) {
                            Ok((new_vout, new_offset)) => {
                                let new_outpoint = OutPoint { txid, vout: new_vout };
                                let occ_count = occupancy.count.max(1);

                                inscription_outpoint_to_offsets
                                    .entry(new_outpoint)
                                    .or_default()
                                    .entry(new_offset)
                                    .and_modify(|occ| {
                                        occ.count = occ.count.saturating_add(occ_count);
                                    })
                                    .or_insert(OccupancyState {
                                        first_inscription: occupancy.first_inscription,
                                        initial_cursed_or_vindicated: occupancy.initial_cursed_or_vindicated,
                                        count: occ_count,
                                    });

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

                        // Determine whether this satpoint was already occupied before this inscription.
                        let offsets_map = inscription_outpoint_to_offsets.entry(location.outpoint).or_default();
                        let had_previous_at_location = offsets_map.contains_key(&location.offset);

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

                        // Track the initial cursed/vindicated flag used for persistence.
                        let mut base_cursed_for_location = false;

                        if is_p2tr_only {
                            let pointer_raw = inscription_template.pointer_value;
                        let loc_key = (location.outpoint, location.offset);

                        // Satpoint-based multiplicity, using persisted OccupancyState count
                        // plus per-tx increments to decide reinscription for BRC-20 gating.
                        let (base_count_from_db, initial_flag_db) = offsets_map
                            .get(&location.offset)
                            .map(|state| (u32::from(state.count.max(1)), state.initial_cursed_or_vindicated))
                            .unwrap_or((0, true));

                        let in_tx_count = per_sat_counts.get(&loc_key).copied().unwrap_or(0);
                        let total_before = base_count_from_db + in_tx_count;
                        let per_sat_reinscription = total_before > 0;
                        per_sat_counts.insert(loc_key, in_tx_count.saturating_add(1));
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
                                // Potential reinscription curse keyed by the pre-fee
                                // input offset, mirroring ord/OPI behaviour.
                                if let Some(global_input_offset) = inputs_cum_prefee.get(input_index).copied() {
                                    if is_reinscription(&inscribed_offsets, global_input_offset) {
                                        curse = Some(Curse::Reinscription);
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
                            base_cursed_for_location = base_cursed;

                            // BRC-20-specific cursed flag:
                            //  - For Litecoin (ltc-20), only pre-jubilee curses should be treated
                            //    as BRC-20-invalid (post-jubilee they are "vindicated").
                            //  - For all other coins (including Bitcoin), preserve existing
                            //    behaviour and treat any curse as BRC-20-invalid.
                            let cursed_for_brc20 = compute_cursed_for_brc20(coin, base_cursed, jubilant);

                            let vindicated = base_cursed && jubilant;
                            let unbound = input_value == 0
                                || matches!(curse, Some(Curse::UnrecognizedEvenField))
                                || inscription_template.unrecognized_even_field;

                            if let Some(global_input_offset) = inputs_cum_prefee.get(input_index).copied() {
                                // ord uses the pointer-adjusted offset (if present) for reinscription flag/insertion
                                let target_offset = inscription_template.pointer_value.unwrap_or(global_input_offset);

                                inscription_template.reinscription = inscribed_offsets.contains_key(&target_offset);

                                let (initial_flag, count) = increment_inscription_count(
                                    &mut inscribed_offsets,
                                    target_offset,
                                    base_cursed,
                                    Some(inscription_template.genesis),
                                );

                                if log_this_tx {
                                    eprintln!(
                                        concat!(
                                            "[DEBUG_TX] new-inscr tx={} input={} base={} ptr={:?} ",
                                            "curse={:?} base_cursed={} initial_flag={} count={} owner_opret={} location_offset={}"
                                        ),
                                        txid,
                                        input_index,
                                        target_offset,
                                        pointer_raw,
                                        curse,
                                        base_cursed,
                                        initial_flag,
                                        count,
                                        inscription_template.owner == *OP_RETURN_HASH,
                                        location.offset,
                                    );
                                }
                            }

                            inscription_template.cursed_for_brc20 = cursed_for_brc20;
                            inscription_template.unbound = unbound;
                            inscription_template.vindicated = vindicated;
                        }

                        // Persist multiplicity and initial cursed/vindicated flag for this location.
                        if !had_previous_at_location {
                            offsets_map.insert(
                                location.offset,
                                OccupancyState::new(inscription_template.genesis, base_cursed_for_location),
                            );
                        } else {
                            offsets_map.entry(location.offset).and_modify(|occ| {
                                occ.count = occ.count.saturating_add(1);
                            });
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
                .or_insert_with(|| OccupancyState::from_legacy(false, 1));
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

    fn load_inscription_outpoint_to_offsets(server: &Server, outpoints: Vec<OutPoint>) -> HashMap<OutPoint, BTreeMap<u64, OccupancyState>> {
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
