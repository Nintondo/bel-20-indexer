use crate::Fixed128;

use super::{proto::*, structs::*, *};

type Tickers = HashSet<OriginalTokenTick>;
type Users = HashSet<(FullHash, OriginalTokenTick)>;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum HistoryTokenAction {
    Deploy {
        tick: OriginalTokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Mint {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    DeployTransfer {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Send {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        sender: FullHash,
        txid: Txid,
        vout: u32,
    },
}

impl HistoryTokenAction {
    pub fn tick(&self) -> OriginalTokenTick {
        match self {
            HistoryTokenAction::Deploy { tick, .. }
            | HistoryTokenAction::Mint { tick, .. }
            | HistoryTokenAction::DeployTransfer { tick, .. }
            | HistoryTokenAction::Send { tick, .. } => *tick,
        }
    }

    pub fn recipient(&self) -> FullHash {
        match self {
            HistoryTokenAction::Mint { recipient, .. } => *recipient,
            HistoryTokenAction::DeployTransfer { recipient, .. } => *recipient,
            HistoryTokenAction::Send { recipient, .. } => *recipient,
            HistoryTokenAction::Deploy { recipient, .. } => *recipient,
        }
    }

    pub fn sender(&self) -> Option<FullHash> {
        match self {
            HistoryTokenAction::Send { sender, .. } => Some(*sender),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct TokenCache {
    /// All tokens. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub tokens: HashMap<LowerCaseTokenTick, TokenMeta>,

    /// All token accounts. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub token_accounts: HashMap<AddressToken, TokenBalance>,

    /// All token actions that are not validated yet but just parsed.
    pub token_actions: Vec<TokenAction>,

    /// All transfer actions. Used to check if a transfer is valid. Used like cache.
    pub all_transfers: HashMap<Location, TransferProtoDB>,

    /// All transfer actions that are valid. Used to write to the db.
    pub valid_transfers: BTreeMap<Location, (FullHash, TransferProtoDB)>,
}
impl TokenCache {
    fn try_parse(content_type: &str, content: &[u8]) -> Result<Brc4, Brc4ParseErr> {
        match content_type.split(';').nth(0) {
            Some("text/plain" | "application/json") => {
                let Ok(data) = String::from_utf8(content.to_vec()) else {
                    return Err(Brc4ParseErr::InvalidUtf8);
                };

                let brc4 = match serde_json::from_str::<Brc4>(&data) {
                    Ok(b) => b,
                    Err(error) => match error.to_string().as_str() {
                        "Invalid decimal: empty" => return Err(Brc4ParseErr::DecimalEmpty),
                        "Invalid decimal: overflow from too many digits" => {
                            return Err(Brc4ParseErr::DecimalOverflow)
                        }
                        "value cannot start from + or -" => {
                            return Err(Brc4ParseErr::DecimalPlusMinus)
                        }
                        "value cannot start or end with ." => {
                            return Err(Brc4ParseErr::DecimalDotStartEnd)
                        }
                        "value cannot contain spaces" => return Err(Brc4ParseErr::DecimalSpaces),
                        "invalid digit found in string" => return Err(Brc4ParseErr::InvalidDigit),
                        msg => {
                            // eprintln!("ERR: {msg:?}");
                            return Err(Brc4ParseErr::Unknown(msg.to_string()));
                        }
                    },
                };

                //todo check match network and token
                match &brc4 {
                    Brc4::Mint {
                        proto: MintProto::Bel20 { amt, .. },
                    } if !amt.is_zero() => Ok(brc4),
                    Brc4::Deploy {
                        proto: DeployProto::Bel20 { dec, lim, max, .. },
                    } if *dec <= DeployProto::MAX_DEC
                        && !lim.unwrap_or(*max).is_zero()
                        && !max.is_zero() =>
                    {
                        Ok(brc4)
                    }
                    Brc4::Transfer {
                        proto: TransferProto::Bel20 { amt, .. },
                    } if !amt.is_zero() => Ok(brc4),
                    _ => Err(Brc4ParseErr::WrongProtocol),
                }
            }
            _ => Err(Brc4ParseErr::WrongContentType),
        }
    }

    /// Parses token action from the InscriptionTemplate.
    pub fn parse_token_action(
        &mut self,
        inc: &InscriptionTemplate,
        height: u32,
        created: u32,
    ) -> Option<TransferProto> {
        // skip to not add invalid token creation in token_cache
        if inc.owner.is_op_return_hash() || inc.leaked {
            return None;
        }

        let brc4 = match Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?) {
            Ok(ok) => ok,
            Err(err) => {
                warn!("Error occurred while parse token {:?}", err);
                return None;
            }
        };

        match brc4 {
            Brc4::Deploy { proto } => {
                match proto {
                    DeployProto::Bel20 {
                        tick,
                        max,
                        lim,
                        dec,
                    } => self.token_actions.push(TokenAction::Deploy {
                        genesis: inc.genesis,
                        proto: DeployProtoDB {
                            tick,
                            max,
                            lim: lim.unwrap_or(max),
                            dec,
                            supply: Fixed128::ZERO,
                            transfer_count: 0,
                            mint_count: 0,
                            height,
                            created,
                            deployer: inc.owner,
                            transactions: 1,
                        },
                        owner: inc.owner,
                    }),
                };
            }
            Brc4::Mint { proto } => {
                self.token_actions.push(TokenAction::Mint {
                    owner: inc.owner,
                    proto,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
            }
            Brc4::Transfer { proto } => {
                self.token_actions.push(TokenAction::Transfer {
                    location: inc.location,
                    owner: inc.owner,
                    proto: proto.clone(),
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
                self.all_transfers.insert(
                    inc.location,
                    TransferProtoDB::from_proto(proto.clone(), height),
                );
                return Some(proto);
            }
        };

        None
    }

    pub fn transferred(
        &mut self,
        transfer_location: Location,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    ) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location,
            recipient,
            txid,
            vout,
        });
    }

    pub fn burned_transfer(&mut self, location: Location, txid: Txid, vout: u32) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location: location,
            recipient: *OP_RETURN_HASH,
            txid,
            vout,
        });
    }

    pub fn load_tokens_data(&mut self, db: &DB) -> anyhow::Result<()> {
        let (tickers, users) = self.fill_tickers_and_users();

        let lower_case_ticks: Vec<_> = tickers.iter().map(LowerCaseTokenTick::from).collect();
        self.tokens = db
            .token_to_meta
            .multi_get(lower_case_ticks.iter())
            .into_iter()
            .zip(lower_case_ticks)
            .filter_map(|(v, k)| v.map(|x| (k, TokenMeta::from(x))))
            .collect::<HashMap<_, _>>();

        let keys: Vec<_> = users
            .into_iter()
            .filter_map(|(address, tick)| {
                Some(AddressToken {
                    address,
                    token: self.tokens.get(&tick.into())?.proto.tick,
                })
            })
            .collect();

        self.token_accounts = db.load_token_accounts(keys);

        Ok(())
    }

    fn fill_tickers_and_users(&mut self) -> (Tickers, Users) {
        let mut tickers: Tickers = HashSet::new();
        let mut users: Users = HashSet::new();

        for action in &self.token_actions {
            match action {
                TokenAction::Deploy {
                    proto: DeployProtoDB { tick, .. },
                    ..
                } => {
                    // Load ticks because we need to check if tick is deployed
                    tickers.insert(*tick);
                }
                TokenAction::Mint {
                    owner,
                    proto: MintProto::Bel20 { tick, .. },
                    ..
                } => {
                    tickers.insert(*tick);
                    users.insert((*owner, *tick));
                }
                TokenAction::Transfer {
                    owner,
                    proto: TransferProto::Bel20 { tick, .. },
                    ..
                } => {
                    tickers.insert(*tick);
                    users.insert((*owner, *tick));
                }
                TokenAction::Transferred {
                    transfer_location,
                    recipient,
                    ..
                } => {
                    let valid_transfer = self.valid_transfers.get(transfer_location);
                    let proto = self
                        .all_transfers
                        .get(transfer_location)
                        .map(|x| Some(x.clone()))
                        .unwrap_or_else(|| {
                            valid_transfer.map(|x| Some(x.1.clone())).unwrap_or(None)
                        });
                    if let Some(TransferProtoDB { tick, .. }) = proto {
                        if !recipient.is_op_return_hash() {
                            users.insert((*recipient, tick));
                        }

                        if let Some(transfer) = valid_transfer {
                            users.insert((transfer.0, tick));
                        }
                        tickers.insert(tick);
                    }
                }
            }
        }
        (tickers, users)
    }

    pub fn process_token_actions(
        &mut self,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
        holders: &Holders,
    ) -> Vec<HistoryTokenAction> {
        let mut history = vec![];

        for action in self.token_actions.drain(..) {
            match action {
                TokenAction::Deploy {
                    genesis,
                    proto,
                    owner,
                } => {
                    let DeployProtoDB {
                        tick,
                        max,
                        lim,
                        dec,
                        ..
                    } = proto.clone();
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.tokens.entry(tick.into())
                    {
                        e.insert(TokenMeta { genesis, proto });

                        history.push(HistoryTokenAction::Deploy {
                            tick,
                            max,
                            lim,
                            dec,
                            recipient: owner,
                            txid: genesis.txid,
                            vout: genesis.index,
                        });

                        if let Some(x) = reorg_cache.as_ref() {
                            x.lock().added_deployed_token(tick);
                        }
                    }
                }
                TokenAction::Mint {
                    owner,
                    proto,
                    txid,
                    vout,
                } => {
                    let MintProto::Bel20 { tick, amt } = proto;
                    let Some(token) = self.tokens.get_mut(&tick.into()) else {
                        continue;
                    };
                    let DeployProtoDB {
                        max,
                        lim,
                        dec,
                        supply,
                        mint_count,
                        transactions,
                        tick,
                        ..
                    } = &mut token.proto;

                    if amt.scale() > *dec {
                        continue;
                    }

                    if Fixed128::from(*lim) < amt {
                        continue;
                    }

                    if *supply == Fixed128::from(*max) {
                        continue;
                    }
                    let amt = amt.min(Fixed128::from(*max) - *supply);
                    *supply += amt;
                    *transactions += 1;

                    let key = AddressToken {
                        address: owner,
                        token: *tick,
                    };

                    holders.increase(
                        &key,
                        self.token_accounts
                            .get(&key)
                            .unwrap_or(&TokenBalance::default()),
                        amt,
                    );
                    self.token_accounts.entry(key.clone()).or_default().balance += amt;
                    *mint_count += 1;

                    history.push(HistoryTokenAction::Mint {
                        tick: *tick,
                        amt,
                        recipient: key.address,
                        txid,
                        vout,
                    });

                    if let Some(x) = reorg_cache.as_ref() {
                        x.lock().added_minted_token(key, amt);
                    }
                }
                TokenAction::Transfer {
                    owner,
                    location,
                    proto,
                    txid,
                    vout,
                } => {
                    let Some(data) = self.all_transfers.remove(&location) else {
                        // skip cause is it transfer already spent
                        continue;
                    };

                    let TransferProto::Bel20 { tick, amt } = proto;

                    let Some(token) = self.tokens.get_mut(&tick.into()) else {
                        continue;
                    };
                    let DeployProtoDB {
                        transfer_count,
                        dec,
                        transactions,
                        tick,
                        ..
                    } = &mut token.proto;

                    if amt.scale() > *dec {
                        // skip wrong protocol
                        continue;
                    }

                    let key = AddressToken {
                        address: owner,
                        token: *tick,
                    };
                    let Some(account) = self.token_accounts.get_mut(&key) else {
                        continue;
                    };

                    if amt > account.balance {
                        continue;
                    }

                    if let Some(x) = reorg_cache.as_ref() {
                        x.lock().added_transfer_token(location, key.clone(), amt);
                    }

                    account.balance -= amt;
                    account.transfers_count += 1;
                    account.transferable_balance += amt;

                    history.push(HistoryTokenAction::DeployTransfer {
                        tick: *tick,
                        amt,
                        recipient: key.address,
                        txid,
                        vout,
                    });

                    self.valid_transfers.insert(location, (key.address, data));
                    *transfer_count += 1;
                    *transactions += 1;
                }
                TokenAction::Transferred {
                    transfer_location,
                    recipient,
                    txid,
                    vout,
                } => {
                    let Some((sender, TransferProtoDB { tick, amt, height })) =
                        self.valid_transfers.remove(&transfer_location)
                    else {
                        // skip cause transfer has been already spent
                        continue;
                    };

                    let token = self.tokens.get_mut(&tick.into()).expect("Tick must exist");

                    let DeployProtoDB {
                        transactions, tick, ..
                    } = &mut token.proto;

                    let old_key = AddressToken {
                        address: sender,
                        token: *tick,
                    };

                    let old_account = self.token_accounts.get_mut(&old_key).unwrap();
                    if old_account.transfers_count == 0 || old_account.transferable_balance < amt {
                        panic!("Invalid transfer sender balance");
                    }

                    holders.decrease(&old_key, old_account, amt);
                    old_account.transfers_count -= 1;
                    old_account.transferable_balance -= amt;
                    *transactions += 1;

                    if !recipient.is_op_return_hash() {
                        let recipient_key = AddressToken {
                            address: recipient,
                            token: *tick,
                        };

                        holders.increase(
                            &recipient_key,
                            self.token_accounts
                                .get(&recipient_key)
                                .unwrap_or(&TokenBalance::default()),
                            amt,
                        );

                        self.token_accounts
                            .entry(recipient_key)
                            .or_default()
                            .balance += amt;
                    }

                    history.push(HistoryTokenAction::Send {
                        amt,
                        tick: *tick,
                        recipient,
                        sender,
                        txid,
                        vout,
                    });

                    if let Some(x) = reorg_cache.as_ref() {
                        x.lock().removed_transfer_token(
                            AddressLocation {
                                address: sender,
                                location: transfer_location,
                            },
                            TransferProtoDB {
                                tick: *tick,
                                amt,
                                height,
                            },
                            recipient,
                        );
                    }
                }
            }
        }

        history
    }
}
