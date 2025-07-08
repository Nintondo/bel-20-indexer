use super::{proto::*, structs::*, *};

type Tickers = HashSet<LowerCaseTokenTick>;
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
    pub fn load(prevouts: &HashMap<OutPoint, TxOut>, db: &DB) -> Self {
        let mut token_cache = Self::default();

        let transfers_to_remove: HashSet<_> = prevouts
            .iter()
            .map(|(k, v)| AddressOutPoint {
                address: v.script_pubkey.compute_script_hash(),
                outpoint: *k,
            })
            .collect();

        token_cache.valid_transfers.extend(db.load_transfers(&transfers_to_remove));

        token_cache.all_transfers = token_cache.valid_transfers.iter().map(|(location, (_, proto))| (*location, proto.clone())).collect();

        token_cache
    }

    fn try_parse(content_type: &str, content: &[u8]) -> Result<Brc4, Brc4ParseErr> {
        // Dogecoin wonky bugfix
        if *BLOCKCHAIN == Blockchain::Dogecoin {
            if !content_type.starts_with("text/plain") && !content_type.starts_with("application/json") {
                return Err(Brc4ParseErr::WrongContentType);
            }
        } else {
            let Some("text/plain" | "application/json") = content_type.split(';').nth(0) else {
                return Err(Brc4ParseErr::WrongContentType);
            };
        }

        let Ok(data) = String::from_utf8(content.to_vec()) else {
            return Err(Brc4ParseErr::InvalidUtf8);
        };

        let data = serde_json::from_str::<serde_json::Value>(&data).map_err(|_| Brc4ParseErr::WrongProtocol)?;

        let brc4 = serde_json::from_str::<Brc4>(&serde_json::to_string(&data).map_err(|_| Brc4ParseErr::WrongProtocol)?).map_err(|error| match error.to_string().as_str() {
            "Invalid decimal: empty" => Brc4ParseErr::DecimalEmpty,
            "Invalid decimal: overflow from too many digits" => Brc4ParseErr::DecimalOverflow,
            "value cannot start from + or -" => Brc4ParseErr::DecimalPlusMinus,
            "value cannot start or end with ." => Brc4ParseErr::DecimalDotStartEnd,
            "value cannot contain spaces" => Brc4ParseErr::DecimalSpaces,
            "invalid digit found in string" => Brc4ParseErr::InvalidDigit,
            msg => Brc4ParseErr::Unknown(msg.to_string()),
        })?;

        match &brc4 {
            Brc4::Mint { proto } => {
                let v = proto.value().map_err(|_| Brc4ParseErr::WrongProtocol)?;
                if !v.amt.is_zero() {
                    Ok(brc4)
                } else {
                    Err(Brc4ParseErr::WrongProtocol)
                }
            }
            Brc4::Transfer { proto } => {
                let v = proto.value().map_err(|_| Brc4ParseErr::WrongProtocol)?;
                if !v.amt.is_zero() {
                    Ok(brc4)
                } else {
                    Err(Brc4ParseErr::WrongProtocol)
                }
            }
            Brc4::Deploy { proto } => {
                let v = proto.value().map_err(|_| Brc4ParseErr::WrongProtocol)?;
                if v.dec <= DeployProto::MAX_DEC && !v.lim.unwrap_or(v.max).is_zero() && !v.max.is_zero() {
                    Ok(brc4)
                } else {
                    Err(Brc4ParseErr::WrongProtocol)
                }
            }
        }
    }

    /// Parses token action from the InscriptionTemplate.
    pub fn parse_token_action(&mut self, inc: &InscriptionTemplate, height: u32, created: u32) -> Option<TransferProto> {
        // skip to not add invalid token creation in token_cache
        if inc.owner.is_op_return_hash() || inc.leaked {
            return None;
        }

        let brc4 = match Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?) {
            Ok(ok) => ok,
            Err(_) => {
                return None;
            }
        };

        match brc4 {
            Brc4::Deploy { proto } => {
                let v = proto.value().ok()?;

                self.token_actions.push(TokenAction::Deploy {
                    genesis: inc.genesis,
                    proto: DeployProtoDB {
                        tick: v.tick,
                        max: v.max,
                        lim: v.lim.unwrap_or(v.max),
                        dec: v.dec,
                        supply: Fixed128::ZERO,
                        transfer_count: 0,
                        mint_count: 0,
                        height,
                        created,
                        deployer: inc.owner,
                        transactions: 1,
                    },
                    owner: inc.owner,
                })
            }
            Brc4::Mint { proto } => {
                self.token_actions.push(TokenAction::Mint {
                    owner: inc.owner,
                    proto: proto.value().ok()?,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
            }
            Brc4::Transfer { proto } => {
                self.token_actions.push(TokenAction::Transfer {
                    location: inc.location,
                    owner: inc.owner,
                    proto: proto.value().ok()?,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
                self.all_transfers.insert(inc.location, TransferProtoDB::from_proto(proto.clone(), height).ok()?);
                return Some(proto);
            }
        };

        None
    }

    pub fn transferred(&mut self, transfer_location: Location, recipient: FullHash, txid: Txid, vout: u32) {
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

        self.tokens = db
            .token_to_meta
            .multi_get_kv(tickers.iter(), false)
            .into_iter()
            .map(|(k, v)| (k.clone(), TokenMeta::from(v)))
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
                    tickers.insert((*tick).into());
                }
                TokenAction::Mint {
                    owner,
                    proto: MintProtoWrapper { tick, .. },
                    ..
                } => {
                    tickers.insert((*tick).into());
                    users.insert((*owner, *tick));
                }
                TokenAction::Transfer {
                    owner,
                    proto: MintProtoWrapper { tick, .. },
                    ..
                } => {
                    tickers.insert((*tick).into());
                    users.insert((*owner, *tick));
                }
                TokenAction::Transferred { transfer_location, recipient, .. } => {
                    let valid_transfer = self.valid_transfers.get(transfer_location);
                    let proto = self
                        .all_transfers
                        .get(transfer_location)
                        .map(|x| Some(x.clone()))
                        .unwrap_or_else(|| valid_transfer.map(|x| Some(x.1.clone())).unwrap_or(None));
                    if let Some(TransferProtoDB { tick, .. }) = proto {
                        if !recipient.is_op_return_hash() {
                            users.insert((*recipient, tick));
                        }

                        if let Some(transfer) = valid_transfer {
                            users.insert((transfer.0, tick));
                        }
                        tickers.insert(tick.into());
                    }
                }
            }
        }
        (tickers, users)
    }

    pub fn process_token_actions(&mut self, holders: &Holders) -> Vec<HistoryTokenAction> {
        let mut history = vec![];

        for action in self.token_actions.drain(..) {
            match action {
                TokenAction::Deploy { genesis, proto, owner } => {
                    let DeployProtoDB { tick, max, lim, dec, .. } = proto.clone();
                    if let std::collections::hash_map::Entry::Vacant(e) = self.tokens.entry(tick.into()) {
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
                    }
                }
                TokenAction::Mint { owner, proto, txid, vout } => {
                    let MintProtoWrapper { tick, amt } = proto;
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

                    let key = AddressToken { address: owner, token: *tick };

                    holders.increase(&key, self.token_accounts.get(&key).unwrap_or(&TokenBalance::default()), amt);
                    self.token_accounts.entry(key).or_default().balance += amt;
                    *mint_count += 1;

                    history.push(HistoryTokenAction::Mint {
                        tick: *tick,
                        amt,
                        recipient: key.address,
                        txid,
                        vout,
                    });
                }
                TokenAction::Transfer {
                    owner,
                    location,
                    proto,
                    txid,
                    vout,
                } => {
                    let Some(mut data) = self.all_transfers.remove(&location) else {
                        // skip cause is it transfer already spent
                        continue;
                    };

                    let MintProtoWrapper { tick, amt } = proto;

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

                    data.tick = *tick;

                    if amt.scale() > *dec {
                        // skip wrong protocol
                        continue;
                    }

                    let key = AddressToken { address: owner, token: *tick };
                    let Some(account) = self.token_accounts.get_mut(&key) else {
                        continue;
                    };

                    if amt > account.balance {
                        continue;
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
                    let Some((sender, TransferProtoDB { tick, amt, .. })) = self.valid_transfers.remove(&transfer_location) else {
                        // skip cause transfer has been already spent
                        continue;
                    };

                    let token = self.tokens.get_mut(&tick.into()).expect("Tick must exist");

                    let DeployProtoDB { transactions, tick, .. } = &mut token.proto;

                    let old_key = AddressToken { address: sender, token: *tick };

                    let old_account = self.token_accounts.get_mut(&old_key).unwrap();
                    if old_account.transfers_count == 0 || old_account.transferable_balance < amt {
                        panic!("Invalid transfer sender balance");
                    }

                    holders.decrease(&old_key, old_account, amt);
                    old_account.transfers_count -= 1;
                    old_account.transferable_balance -= amt;
                    *transactions += 1;

                    if !recipient.is_op_return_hash() {
                        let recipient_key = AddressToken { address: recipient, token: *tick };

                        holders.increase(&recipient_key, self.token_accounts.get(&recipient_key).unwrap_or(&TokenBalance::default()), amt);

                        self.token_accounts.entry(recipient_key).or_default().balance += amt;
                    }

                    history.push(HistoryTokenAction::Send {
                        amt,
                        tick: *tick,
                        recipient,
                        sender,
                        txid,
                        vout,
                    });
                }
            }
        }

        history
    }
}
