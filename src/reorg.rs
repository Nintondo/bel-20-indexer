use super::*;

pub const REORG_CACHE_MAX_LEN: usize = 30;

enum TokenHistoryEntry {
    RemoveDeployed(TokenTick),
    /// Second arg `Fixed128` is amount of mint to remove. We need to decrease user balance + mint count + total supply of deploy
    RemoveMint(AddressToken, Fixed128),
    /// Second arg `Fixed128` is amount of transfer to remove. We need to decrease user balance, transfers_count, transfers_amount + transfer count of deploy
    RemoveTransfer(Location, AddressToken, Fixed128),
    /// Key and value of removed valid transfer
    RestoreTransferred(AddressLocation, TransferProtoDB, FullHash),
    RemoveHistory(AddressTokenId),
    RestorePrevout(OutPoint, TxOut),
}

struct ReorgHistoryBlock {
    token_history: Vec<TokenHistoryEntry>,
    last_history_id: u64,
    block_header: BlockHeader,
}

impl ReorgHistoryBlock {
    fn new(block_header: BlockHeader, last_history_id: u64) -> Self {
        Self {
            last_history_id,
            block_header,
            token_history: vec![],
        }
    }
}

pub struct ReorgCache {
    blocks: BTreeMap<u32, ReorgHistoryBlock>,
    len: usize,
}

impl ReorgCache {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            len: REORG_CACHE_MAX_LEN,
        }
    }

    pub fn get_blocks_headers(&self) -> Vec<BlockHeader> {
        self.blocks
            .values()
            .map(|x| x.block_header.clone())
            .collect()
    }

    pub fn new_block(&mut self, block_header: BlockHeader, last_history_id: u64) {
        if self.blocks.len() == self.len {
            self.blocks.pop_first();
        }
        self.blocks.insert(
            block_header.number,
            ReorgHistoryBlock::new(block_header, last_history_id),
        );
    }

    pub fn added_deployed_token(&mut self, tick: TokenTick) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RemoveDeployed(tick));
    }

    pub fn added_minted_token(&mut self, token: AddressToken, amount: Fixed128) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RemoveMint(token, amount));
    }

    pub fn added_history(&mut self, key: AddressTokenId) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RemoveHistory(key));
    }

    pub fn removed_prevout(&mut self, key: OutPoint, value: TxOut) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RestorePrevout(key, value));
    }

    pub fn added_transfer_token(
        &mut self,
        location: Location,
        token: AddressToken,
        amount: Fixed128,
    ) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RemoveTransfer(location, token, amount));
    }

    pub fn removed_transfer_token(
        &mut self,
        key: AddressLocation,
        value: TransferProtoDB,
        recipient: FullHash,
    ) {
        self.blocks
            .last_entry()
            .unwrap()
            .get_mut()
            .token_history
            .push(TokenHistoryEntry::RestoreTransferred(key, value, recipient));
    }

    pub fn restore(&mut self, server: &Server, block_height: u32) -> anyhow::Result<()> {
        while !self.blocks.is_empty() && block_height <= *self.blocks.last_key_value().unwrap().0 {
            let (height, data) = self.blocks.pop_last().anyhow()?;

            server.db.last_block.set((), height - 1);
            server.db.last_history_id.set((), data.last_history_id);
            server.db.block_hashes.remove(height);

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
                        TokenHistoryEntry::RemoveDeployed(tick) => {
                            to_remove_deployed.push(LowerCaseTick::from(tick));
                        }
                        TokenHistoryEntry::RemoveMint(receiver, amt) => {
                            to_update_deployed
                                .push(DeployedUpdate::Mint(receiver.token.clone(), amt));
                            to_remove_minted.push((receiver, amt));
                        }
                        TokenHistoryEntry::RemoveTransfer(location, receiver, amt) => {
                            to_update_deployed
                                .push(DeployedUpdate::Transfer(receiver.token.clone()));
                            to_remove_transfer.push((location, receiver, amt));
                        }
                        TokenHistoryEntry::RestoreTransferred(key, value, recipient) => {
                            to_update_deployed.push(DeployedUpdate::Transferred(value.tick.into()));
                            to_restore_transferred.push((key, value, recipient));
                        }
                        TokenHistoryEntry::RemoveHistory(key) => {
                            to_remove_history.push(key);
                        }
                        TokenHistoryEntry::RestorePrevout(key, value) => {
                            to_restore_prevout.push((key, value));
                        }
                    }
                }

                let keys_to_remove = server
                    .db
                    .address_token_to_history
                    .multi_get(to_remove_history.iter())
                    .into_iter()
                    .flatten()
                    .map(|x| x.action.outpoint());

                server.db.outpoint_to_event.remove_batch(keys_to_remove);

                server
                    .db
                    .address_token_to_history
                    .remove_batch(to_remove_history.into_iter());
                server.db.prevouts.extend(to_restore_prevout.into_iter());

                {
                    let deploy_keys = to_update_deployed
                        .iter()
                        .map(|x| match x {
                            DeployedUpdate::Mint(tick, _)
                            | DeployedUpdate::Transfer(tick)
                            | DeployedUpdate::Transferred(tick) => tick.clone(),
                        })
                        .unique()
                        .collect_vec();

                    let deploys = server
                        .db
                        .token_to_meta
                        .multi_get(deploy_keys.iter())
                        .into_iter()
                        .zip(deploy_keys)
                        .map(|(v, k)| v.map(|x| (k, x)))
                        .collect::<Option<HashMap<_, _>>>()
                        .anyhow_with("Some of deploys is not found")?;

                    let updated_values = to_update_deployed.into_iter().rev().map(|x| match x {
                        DeployedUpdate::Mint(tick, amt) => {
                            let mut meta = deploys.get(&tick.clone()).unwrap().clone();
                            let DeployProtoDB {
                                supply,
                                mint_count,
                                transactions,
                                ..
                            } = &mut meta.proto;
                            *supply -= amt;
                            *mint_count -= 1;
                            *transactions -= 1;
                            (tick, meta)
                        }
                        DeployedUpdate::Transfer(tick) => {
                            let mut meta = deploys.get(&tick.clone()).unwrap().clone();
                            let DeployProtoDB {
                                transfer_count,
                                transactions,
                                ..
                            } = &mut meta.proto;
                            *transfer_count -= 1;
                            *transactions -= 1;
                            (tick, meta)
                        }
                        DeployedUpdate::Transferred(tick) => {
                            let mut meta = deploys.get(&tick).unwrap().clone();
                            let DeployProtoDB { transactions, .. } = &mut meta.proto;
                            *transactions -= 1;
                            (tick, meta)
                        }
                    });

                    server.db.token_to_meta.extend(updated_values);
                    server
                        .db
                        .token_to_meta
                        .remove_batch(to_remove_deployed.into_iter());
                }

                let mut accounts = {
                    let keys = to_remove_minted
                        .iter()
                        .map(|x| x.0.clone())
                        .chain(to_remove_transfer.iter().map(|x| x.1.clone()))
                        .chain(to_restore_transferred.iter().flat_map(|(k, v, recipient)| {
                            [
                                AddressToken {
                                    address: k.address,
                                    token: v.tick.into(),
                                },
                                AddressToken {
                                    address: *recipient,
                                    token: v.tick.into(),
                                },
                            ]
                        }))
                        .collect_vec();

                    server
                        .db
                        .address_token_to_balance
                        .multi_get(keys.iter())
                        .into_iter()
                        .zip(keys)
                        .map(|(v, k)| v.map(|x| (k, x)))
                        .collect::<Option<HashMap<_, _>>>()
                        .anyhow_with("Some of accounts is not found")?
                };

                {
                    for (key, amt) in to_remove_minted.into_iter().rev() {
                        let account = accounts.get_mut(&key).unwrap();
                        server.holders.decrease(&key, account, amt);
                        account.balance = account.balance.checked_sub(amt).anyhow()?;
                    }

                    let transfer_locations_to_remove = to_remove_transfer
                        .into_iter()
                        .map(|(location, address, amt)| {
                            if let Some(x) = accounts.get_mut(&address) {
                                x.balance += amt;
                                x.transferable_balance =
                                    x.transferable_balance.checked_sub(amt).expect("Overflow");
                                x.transfers_count -= 1;
                            };

                            AddressLocation {
                                address: address.address,
                                location,
                            }
                        })
                        .collect::<HashSet<_>>();

                    for (k, v, recipient) in &to_restore_transferred {
                        let key = AddressToken {
                            address: k.address,
                            token: v.tick.into(),
                        };

                        let account = accounts.get_mut(&key).unwrap();

                        server.holders.increase(&key, account, v.amt);
                        account.transferable_balance += v.amt;
                        account.transfers_count += 1;

                        if !recipient.is_op_return_hash() {
                            let key = AddressToken {
                                address: *recipient,
                                token: v.tick.into(),
                            };

                            let account = accounts.get_mut(&key).unwrap();

                            server.holders.decrease(&key, account, v.amt);
                            account.balance = account.balance.checked_sub(v.amt).anyhow()?;
                        }
                    }

                    server
                        .db
                        .address_token_to_balance
                        .extend(accounts.into_iter());
                    server.db.address_location_to_transfer.extend(
                        to_restore_transferred
                            .into_iter()
                            .map(|x| (x.0, x.1))
                            .filter(|x| !transfer_locations_to_remove.contains(&x.0)),
                    );
                    server
                        .db
                        .address_location_to_transfer
                        .remove_batch(transfer_locations_to_remove.into_iter());
                }
            }
        }

        Ok(())
    }

    pub fn restore_all(&mut self, server: &Server) -> anyhow::Result<()> {
        let from = self.blocks.first_key_value().map(|x| *x.0);
        let to = self.blocks.last_key_value().map(|x| *x.0);

        warn!("Restoring savepoints from {:?} to {:?}", from, to);
        self.restore(server, 0)
    }
}

enum DeployedUpdate {
    Mint(LowerCaseTick, Fixed128),
    Transfer(LowerCaseTick),
    Transferred(LowerCaseTick),
}
