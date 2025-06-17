use super::*;

pub enum ProcessedData {
    Info {
        block_number: u32,
        block_info: BlockInfo,
    },
    Prevouts {
        to_write: HashMap<OutPoint, TxOut>,
        to_remove: Vec<OutPoint>,
    },
    FullHash {
        addresses: Vec<(FullHash, String)>,
    },
    Proof {
        block_number: u32,
        block_proof: sha256::Hash,
    },
    History {
        block_number: u32,
        last_history_id: u64,
        history: Vec<(AddressTokenIdDB, HistoryValue)>,
    },
    Tokens {
        metas: Vec<(LowerCaseTokenTick, TokenMetaDB)>,
        balances: Vec<(AddressToken, TokenBalance)>,
        transfers_to_write: Vec<(AddressLocation, TransferProtoDB)>,
        transfers_to_remove: Vec<AddressLocation>,
    },
    InscriptionPartials {
        to_remove: Vec<OutPoint>,
        to_write: Vec<(OutPoint, Partials)>,
    },
    InscriptionOffset {
        to_remove: Vec<OutPoint>,
        to_write: Vec<(OutPoint, HashSet<u64>)>,
    },
}

impl ProcessedData {
    pub fn write(self, db: &DB) {
        match self {
            ProcessedData::Info {
                block_number,
                block_info,
            } => {
                db.last_block.set((), block_number);
                db.block_info.set(block_number, block_info);
            }
            ProcessedData::Prevouts {
                to_write,
                to_remove,
            } => {
                db.prevouts.remove_batch(to_remove.into_iter());
                db.prevouts.extend(to_write);
            }
            ProcessedData::FullHash { addresses } => {
                db.fullhash_to_address.extend(addresses);
            }
            ProcessedData::Proof {
                block_number,
                block_proof,
            } => {
                db.proof_of_history.set(block_number, block_proof);
            }
            ProcessedData::History {
                block_number,
                last_history_id,
                history,
            } => {
                let block_events: Vec<_> = history
                    .iter()
                    .map(|(address_token_id, _)| *address_token_id)
                    .sorted_unstable_by_key(|address_token_id| address_token_id.id)
                    .collect();

                let outpoint_to_event = history.iter().map(|(address_token_id, history_value)| {
                    (history_value.action.outpoint(), address_token_id)
                });

                db.block_events.set(block_number, block_events);
                db.outpoint_to_event.extend(outpoint_to_event);
                db.address_token_to_history.extend(history);
                db.last_history_id.set((), last_history_id);
            }
            ProcessedData::Tokens {
                metas,
                balances,
                transfers_to_write,
                transfers_to_remove,
            } => {
                db.token_to_meta.extend(metas);
                db.address_token_to_balance.extend(balances);
                db.address_location_to_transfer
                    .remove_batch(transfers_to_remove.into_iter());
                db.address_location_to_transfer.extend(transfers_to_write);
            }
            ProcessedData::InscriptionPartials {
                to_remove,
                to_write,
            } => {
                db.outpoint_to_partials.remove_batch(to_remove.into_iter());
                db.outpoint_to_partials.extend(to_write);
            }
            ProcessedData::InscriptionOffset {
                to_remove,
                to_write,
            } => {
                db.outpoint_to_inscription_offsets
                    .remove_batch(to_remove.into_iter());
                db.outpoint_to_inscription_offsets.extend(to_write);
            }
        }
    }
}
