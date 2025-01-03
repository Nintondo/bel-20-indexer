use super::*;

pub const REORG_CACHE_MAX_LEN: usize = 30;

enum TokenHistoryEntry {
    ToRemoveDeployed(TokenTick),
    /// Second arg `Decimal` is amount of mint to remove. We need to decrease user balance + mint count + total supply of deploy
    ToRemoveMint(AddressToken, Decimal),
    /// Second arg `Decimal` is amount of transfer to remove. We need to decrease user balance, transfers_count, transfers_amount + transfer count of deploy
    ToRemoveTransfer(Location, AddressToken, Decimal),
    /// Key and value of removed valid transfer
    ToRestoreTrasferred(AddressLocation, TransferProtoDB, Option<FullHash>),
    ToRemoveHistory(AddressTokenId),
    ToRestorePrevout(OutPoint, TxOut),
}

#[derive(Default)]
struct ReorgHistoryBlock {
    token_history: Vec<TokenHistoryEntry>,
    last_history_id: u64,
}

impl ReorgHistoryBlock {
    fn new(last_history_id: u64) -> Self {
        Self {
            last_history_id,
            ..Default::default()
        }
    }
}

pub struct ReorgCache {
    blocks: BTreeMap<u64, ReorgHistoryBlock>,
    len: usize,
}

impl ReorgCache {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            len: REORG_CACHE_MAX_LEN,
        }
    }

    pub fn new_block(&mut self, block_height: u64, last_history_id: u64) {
        if self.blocks.len() == self.len {
            self.blocks.pop_first();
        }
        self.blocks
            .insert(block_height, ReorgHistoryBlock::new(last_history_id));
    }

    pub fn added_deployed_token(&mut self, tick: TokenTick) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRemoveDeployed(tick));
    }

    pub fn added_minted_token(&mut self, token: AddressToken, amount: Decimal) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRemoveMint(token, amount));
    }

    pub fn added_history(&mut self, key: AddressTokenId) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRemoveHistory(key));
    }

    pub fn removed_prevout(&mut self, key: OutPoint, value: TxOut) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRestorePrevout(key, value));
    }

    pub fn added_transfer_token(
        &mut self,
        location: Location,
        token: AddressToken,
        amount: Decimal,
    ) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRemoveTransfer(location, token, amount));
    }

    pub fn removed_transfer_token(
        &mut self,
        key: AddressLocation,
        value: TransferProto,
        recipient: Option<FullHash>,
    ) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::ToRestoreTrasferred(
                key,
                value.into(),
                recipient,
            ));
    }

    pub fn restore(&mut self, db: &DB, block_height: u64) -> anyhow::Result<()> {
        while !self.blocks.is_empty() && block_height <= *self.blocks.last_key_value().unwrap().0 {
            let (height, data) = self.blocks.pop_last().anyhow()?;

            db.last_block.set((), height as u64 - 1);
            db.last_history_id.set((), data.last_history_id);
            db.block_hashes.remove(height as u64);

            {
                let mut to_remove_deployed = vec![];
                let mut to_remove_minted = vec![];
                let mut to_update_deployed = vec![];
                let mut to_remove_transfer = vec![];
                let mut to_restore_transferred = vec![];
                let mut to_remove_history = vec![];
                let mut to_restore_prevout = vec![];

                for entry in data.token_history.into_iter().rev() {
                    match entry {
                        TokenHistoryEntry::ToRemoveDeployed(tick) => {
                            to_remove_deployed.push(tick);
                        }
                        TokenHistoryEntry::ToRemoveMint(receiver, amt) => {
                            to_update_deployed.push(DeployedUpdate::Mint(receiver.token, amt));
                            to_remove_minted.push((receiver, amt));
                        }
                        TokenHistoryEntry::ToRemoveTransfer(location, receiver, amt) => {
                            to_update_deployed.push(DeployedUpdate::Transfer(receiver.token));
                            to_remove_transfer.push((location, receiver, amt));
                        }
                        TokenHistoryEntry::ToRestoreTrasferred(key, value, recipient) => {
                            to_restore_transferred.push((key, value, recipient));
                        }
                        TokenHistoryEntry::ToRemoveHistory(key) => {
                            to_remove_history.push(key);
                        }
                        TokenHistoryEntry::ToRestorePrevout(key, value) => {
                            to_restore_prevout.push((key, value));
                        }
                    }
                }

                db.address_token_to_history
                    .remove_batch(to_remove_history.into_iter());
                db.prevouts.extend(to_restore_prevout.into_iter());

                {
                    let deploy_keys = to_update_deployed
                        .iter()
                        .map(|x| match x {
                            DeployedUpdate::Mint(tick, _) | DeployedUpdate::Transfer(tick) => *tick,
                        })
                        .unique()
                        .collect_vec();

                    let deploys = db
                        .token_to_meta
                        .multi_get(deploy_keys.iter())
                        .into_iter()
                        .zip(deploy_keys)
                        .map(|(v, k)| v.map(|x| (k, x)))
                        .collect::<Option<HashMap<_, _>>>()
                        .anyhow_with("Some of deploys is not found")?;

                    let updated_values = to_update_deployed.into_iter().rev().map(|x| match x {
                        DeployedUpdate::Mint(tick, amt) => {
                            let mut meta = deploys.get(&tick).unwrap().clone();
                            let DeployProtoDB {
                                supply, mint_count, ..
                            } = &mut meta.proto;
                            *supply -= amt;
                            *mint_count -= 1;
                            (tick, meta)
                        }
                        DeployedUpdate::Transfer(tick) => {
                            let mut meta = deploys.get(&tick).unwrap().clone();
                            let DeployProtoDB { transfer_count, .. } = &mut meta.proto;
                            *transfer_count -= 1;
                            (tick, meta)
                        }
                    });

                    db.token_to_meta.extend(updated_values);
                    db.token_to_meta
                        .remove_batch(to_remove_deployed.into_iter());
                }

                let mut accounts = {
                    let keys = to_remove_minted
                        .iter()
                        .map(|x| x.0.clone())
                        .chain(to_remove_transfer.iter().map(|x| x.1.clone()))
                        .chain(to_restore_transferred.iter().flat_map(|(k, v, recipient)| {
                            [
                                Some(AddressToken {
                                    address: k.address,
                                    token: v.tick,
                                }),
                                recipient.map(|recipient| AddressToken {
                                    address: recipient,
                                    token: v.tick,
                                }),
                            ]
                            .into_iter()
                            .flatten()
                        }))
                        .collect_vec();

                    db.address_token_to_balance
                        .multi_get(keys.iter())
                        .into_iter()
                        .zip(keys)
                        .map(|(v, k)| v.map(|x| (k, x)))
                        .collect::<Option<HashMap<_, _>>>()
                        .anyhow_with("Some of accounts is not found")?
                };

                {
                    to_remove_minted
                        .into_iter()
                        .rev()
                        .for_each(|(address, amt)| {
                            accounts.get_mut(&address).map(|x| {
                                x.balance -= amt;
                            });
                        });

                    let transfer_locations_to_remove = to_remove_transfer
                        .into_iter()
                        .map(|(location, address, amt)| {
                            accounts.get_mut(&address).map(|x| {
                                x.balance += amt;
                                x.transferable_balance -= amt;
                                x.transfers_count = x.transfers_count.checked_sub(1).unwrap();
                            });
                            AddressLocation {
                                address: address.address,
                                location: location.into(),
                            }
                        })
                        .collect::<HashSet<_>>();

                    to_restore_transferred.iter().for_each(|(k, v, recipient)| {
                        accounts
                            .get_mut(&AddressToken {
                                address: k.address,
                                token: v.tick,
                            })
                            .map(|x| {
                                x.transferable_balance += v.amt;
                                x.transfers_count = x.transfers_count.checked_add(1).unwrap();
                            });
                        if let Some(recipient) = recipient {
                            accounts
                                .get_mut(&AddressToken {
                                    address: *recipient,
                                    token: v.tick,
                                })
                                .unwrap()
                                .balance -= v.amt;
                        }
                    });

                    if accounts.iter().any(|x| {
                        x.1.balance.is_sign_negative()
                            || x.1.transferable_balance.is_sign_negative()
                    }) {
                        anyhow::bail!("Some of accounts is overflowed");
                    }

                    db.address_token_to_balance.extend(accounts.into_iter());
                    db.address_location_to_transfer.extend(
                        to_restore_transferred
                            .into_iter()
                            .map(|x| (x.0, x.1))
                            .filter(|x| !transfer_locations_to_remove.contains(&x.0)),
                    );
                    db.address_location_to_transfer
                        .remove_batch(transfer_locations_to_remove.into_iter());
                }
            }
        }

        Ok(())
    }

    pub fn restore_all(&mut self, db: &DB) -> anyhow::Result<()> {
        let from = self.blocks.first_key_value().map(|x| *x.0);
        let to = self.blocks.last_key_value().map(|x| *x.0);

        warn!("Restoring savepoints from {:?} to {:?}", from, to);
        self.restore(&db, 0)
    }
}

enum DeployedUpdate {
    Mint(TokenTick, Decimal),
    Transfer(TokenTick),
}
