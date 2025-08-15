use crate::reorg::{OrdinalsEntry, TokenHistoryEntry};

use super::*;

pub enum ProcessedData {
    Info {
        block_number: u32,
        block_info: BlockInfo,
        block_proof: sha256::Hash,
    },
    Prevouts {
        to_write: HashMap<OutPoint, TxPrevout>,
        to_remove: Vec<OutPoint>,
    },
    FullHash {
        addresses: Vec<(FullHash, String)>,
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
        to_remove: Vec<(OutPoint, Partials)>,
        to_write: Vec<(OutPoint, Partials)>,
    },
    InscriptionOffset {
        to_remove: Vec<(OutPoint, HashSet<u64>)>,
        to_write: Vec<(OutPoint, HashSet<u64>)>,
    },
}

impl ProcessedData {
    pub fn write(self, server: &Server, reorg_cache: Option<Arc<parking_lot::Mutex<ReorgCache>>>) {
        let mut reorg_cache = reorg_cache.as_ref().map(|x| x.lock());

        match self {
            ProcessedData::Info {
                block_number,
                block_info,
                block_proof,
            } => {
                server.db.last_block.set((), block_number);
                server.db.block_info.set(block_number, block_info);
                server.db.proof_of_history.set(block_number, block_proof);
            }
            ProcessedData::Prevouts { to_write, to_remove } => {
                if let Some(reorg_cache) = reorg_cache.as_mut() {
                    let prevouts = server
                        .db
                        .prevouts
                        .multi_get(to_remove.iter())
                        .into_iter()
                        .zip(to_remove.iter())
                        .map(|(v, k)| (*k, v.unwrap_or_else(|| *to_write.get(k).unwrap())))
                        .collect();

                    reorg_cache.push_ordinals_entry(OrdinalsEntry::RestorePrevouts(prevouts));
                }

                server.db.prevouts.extend(to_write);
                server.db.prevouts.remove_batch(to_remove);
            }
            ProcessedData::FullHash { addresses } => {
                server.db.fullhash_to_address.extend(addresses);
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

                let outpoint_to_event = history
                    .iter()
                    .map(|(address_token_id, history_value)| (history_value.action.outpoint(), address_token_id))
                    .collect_vec();

                let token_id_to_event = history
                    .iter()
                    .map(|(address_token_id, _)| {
                        (
                            TokenId {
                                token: address_token_id.token,
                                id: address_token_id.id,
                            },
                            address_token_id,
                        )
                    })
                    .collect_vec();

                if let Some(reorg_cache) = reorg_cache.as_mut() {
                    reorg_cache.push_token_entry(TokenHistoryEntry::RemoveHistory {
                        height: block_number,
                        last_history_id: server.db.last_history_id.get(()).unwrap_or_default(),
                        outpoint_to_event: outpoint_to_event.iter().map(|x| x.0).collect(),
                        to_remove: history.iter().map(|x| x.0).collect(),
                        token_id_to_event: token_id_to_event.iter().map(|x| x.0).collect(),
                    });
                }

                server.db.token_id_to_event.extend(token_id_to_event);
                server.db.block_events.set(block_number, block_events);
                server.db.last_history_id.set((), last_history_id);
                server.db.outpoint_to_event.extend(outpoint_to_event);
                server.db.address_token_to_history.extend(history);
            }
            ProcessedData::Tokens {
                metas,
                balances,
                transfers_to_write,
                transfers_to_remove,
            } => {
                if let Some(reorg_cache) = reorg_cache.as_mut() {
                    // Deploys
                    {
                        let deploys = server
                            .db
                            .token_to_meta
                            .multi_get_kv(metas.iter().map(|x| &x.0), false)
                            .into_iter()
                            .map(|x| (x.0.clone(), x.1))
                            .collect::<HashMap<_, _>>();

                        let new_deploys = metas.iter().filter(|x| !deploys.contains_key(&x.0)).map(|x| x.0.clone()).collect_vec();

                        reorg_cache.push_token_entry(TokenHistoryEntry::DeploysToRemove(new_deploys));
                    }

                    // Balances
                    {
                        let balances_before = server
                            .db
                            .address_token_to_balance
                            .multi_get_kv(balances.iter().map(|x| &x.0), false)
                            .into_iter()
                            .map(|x| (*x.0, x.1))
                            .collect::<HashMap<_, _>>();

                        let new_balances = balances.iter().filter(|x| !balances_before.contains_key(&x.0)).map(|x| x.0).collect_vec();

                        reorg_cache.push_token_entry(TokenHistoryEntry::BalancesBefore(balances_before.into_iter().collect()));
                        reorg_cache.push_token_entry(TokenHistoryEntry::BalancesToRemove(new_balances));
                    }

                    // Transfers
                    {
                        let to_restore_transfers = server
                            .db
                            .address_location_to_transfer
                            .multi_get_kv(transfers_to_remove.iter(), true)
                            .into_iter()
                            .map(|x| (x.0.clone(), x.1))
                            .collect_vec();
                        let to_remove_transfers = transfers_to_write.iter().map(|x| x.0.clone()).collect_vec();

                        reorg_cache.push_token_entry(TokenHistoryEntry::RestoreTransfers(to_restore_transfers));
                        reorg_cache.push_token_entry(TokenHistoryEntry::RemoveTransfers(to_remove_transfers));
                    }
                }

                server.db.token_to_meta.extend(metas);
                server.db.address_token_to_balance.extend(balances);
                server.db.address_location_to_transfer.remove_batch(transfers_to_remove);
                server.db.address_location_to_transfer.extend(transfers_to_write);
            }
            ProcessedData::InscriptionPartials { to_remove, to_write } => {
                if let Some(reorg_cache) = reorg_cache.as_mut() {
                    reorg_cache.push_ordinals_entry(OrdinalsEntry::RestorePartial(to_remove.clone()));
                    reorg_cache.push_ordinals_entry(OrdinalsEntry::RemovePartials(to_write.iter().map(|x| x.0).collect_vec()));
                }

                server.db.outpoint_to_partials.remove_batch(to_remove.iter().map(|x| x.0));
                server.db.outpoint_to_partials.extend(to_write);
            }
            ProcessedData::InscriptionOffset { to_remove, to_write } => {
                if let Some(reorg_cache) = reorg_cache.as_mut() {
                    reorg_cache.push_ordinals_entry(OrdinalsEntry::RestoreOffsets(to_remove.clone()));
                    reorg_cache.push_ordinals_entry(OrdinalsEntry::RemoveOffsets(to_write.iter().map(|x| x.0).collect_vec()));
                }

                server.db.outpoint_to_inscription_offsets.remove_batch(to_remove.iter().map(|x| x.0));
                server.db.outpoint_to_inscription_offsets.extend(to_write);
            }
        }
    }
}
