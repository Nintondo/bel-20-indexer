use nint_blk::{Coin, CoinType};

use super::{proto::*, structs::*, *};
use std::collections::HashSet;

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

#[derive(Clone)]
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

    pub server: Arc<Server>,
}

impl TokenCache {
    pub fn load(prevouts: &HashMap<OutPoint, TxPrevout>, server: Arc<Server>, runtime: &RuntimeTokenState) -> Self {
        let mut token_cache = Self {
            all_transfers: HashMap::new(),
            server,
            token_accounts: HashMap::new(),
            token_actions: Vec::new(),
            tokens: HashMap::new(),
            valid_transfers: BTreeMap::new(),
        };

        let transfers_to_remove: HashSet<_> = prevouts
            .iter()
            .map(|(k, v)| AddressOutPoint {
                address: v.script_hash,
                outpoint: *k,
            })
            .collect();

        // Use the secondary in-memory index on RuntimeTokenState to load only
        // the transfers relevant to this block, without scanning all
        // valid_transfers or hitting RocksDB.
        for ao in &transfers_to_remove {
            if let Some(locations) = runtime.transfers_by_outpoint.get(ao) {
                for loc in locations {
                    if let Some((addr, proto)) = runtime.valid_transfers.get(loc) {
                        token_cache.valid_transfers.insert(*loc, (*addr, proto.clone()));
                    }
                }
            }
        }

        token_cache.all_transfers = token_cache.valid_transfers.iter().map(|(location, (_, proto))| (*location, proto.clone())).collect();

        token_cache
    }

    pub(crate) fn try_parse(content_type: &str, content: &[u8], coin: CoinType) -> Result<Brc4, Brc4ParseErr> {
        // Dogecoin wonky bugfix
        if coin.name == nint_blk::Dogecoin::NAME {
            if !content_type.starts_with("text/plain") && !content_type.starts_with("application/json") {
                return Err(Brc4ParseErr::WrongContentType);
            }
        } else {
            let Some("text/plain" | "application/json") = content_type.split(';').nth(0) else {
                return Err(Brc4ParseErr::WrongContentType);
            };
        }

        // Validate UTF-8 first to preserve existing error semantics.
        let data = core::str::from_utf8(content).map_err(|_| Brc4ParseErr::InvalidUtf8)?;

        // Cheap byte-level prefilters to skip obviously irrelevant payloads without full JSON work.
        if content.len() < 4 || !content.windows(3).any(|w| w == b"\"p\"") {
            return Err(Brc4ParseErr::WrongProtocol);
        }
        if !content.windows(coin.brc_name.len()).any(|w| w.eq_ignore_ascii_case(coin.brc_name.as_bytes())) {
            return Err(Brc4ParseErr::WrongProtocol);
        }

        #[derive(Serialize, Deserialize)]
        struct Brc4Envelope {
            #[serde(rename = "p")]
            protocol: String,
            #[serde(flatten)]
            inner: Brc4,
        }

        let envelope = serde_json::from_str::<Brc4Envelope>(data).map_err(|error| match error.to_string().as_str() {
            "Invalid decimal: empty" => Brc4ParseErr::DecimalEmpty,
            "Invalid decimal: overflow from too many digits" => Brc4ParseErr::DecimalOverflow,
            "value cannot start from + or -" => Brc4ParseErr::DecimalPlusMinus,
            "value cannot start or end with ." => Brc4ParseErr::DecimalDotStartEnd,
            "value cannot contain spaces" => Brc4ParseErr::DecimalSpaces,
            "invalid digit found in string" => Brc4ParseErr::InvalidDigit,
            _ => Brc4ParseErr::WrongProtocol,
        })?;

        if envelope.protocol != coin.brc_name {
            return Err(Brc4ParseErr::WrongProtocol);
        }

        let brc4 = envelope.inner;

        match &brc4 {
            Brc4::Mint { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Transfer { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Deploy { proto } => {
                let dec_ok = proto.dec <= DeployProto::MAX_DEC;
                // For max=0, allow self_mint tokens regardless of lim presence/value (normalized later)
                // For max>0, require non-zero effective lim
                let ok = if proto.max.is_zero() {
                    proto.self_mint
                } else {
                    !proto.lim.unwrap_or(proto.max).is_zero()
                };
                // Enforce self_mint policy by length:
                //  - 5-byte tickers must be self_mint
                //  - 4-byte tickers must NOT be self_mint
                let tick_len_ok = if proto.tick.len() == 5 { proto.self_mint } else { !proto.self_mint };
                if dec_ok && ok && tick_len_ok {
                    Ok(brc4)
                } else {
                    Err(Brc4ParseErr::WrongProtocol)
                }
            }
            &Brc4::Mint { .. } | &Brc4::Transfer { .. } => Err(Brc4ParseErr::WrongProtocol),
        }
    }

    /// Parses token action from the InscriptionTemplate.
    pub fn parse_token_action(&mut self, inc: &InscriptionTemplate, height: u32, created: u32) -> Option<TransferProto> {
        // skip to not add invalid token creation in token_cache
        if inc.owner.is_op_return_hash() || inc.leaked {
            return None;
        }

        let coin = self.server.indexer.coin;

        // ord/OPI-style gating for BRC20 inscriptions on p2tr-only coins.
        // Make behaviour coin-specific so that:
        //  - BTC (brc-20) keeps existing logic: reject cursed or unbound inscriptions.
        if coin.only_p2tr && (inc.cursed_for_brc20 || inc.unbound) {
            return None;
        }

        let brc4 = match Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?, coin) {
            Ok(ok) => ok,
            Err(_) => {
                return None;
            }
        };

        match brc4 {
            Brc4::Deploy { proto } => {
                let v = proto;

                // Activation and policy checks
                let act = coin.self_mint_activation_height;
                let is_5_byte = v.tick.len() == 5;
                // Policy:
                //  - 5-byte tickers: require activation AND self_mint=true
                //  - 4-byte tickers: self_mint must be false
                if is_5_byte {
                    if !act.map(|h| (height as usize) >= h).unwrap_or(false) {
                        return None;
                    }
                    if !v.self_mint {
                        return None;
                    }
                } else if v.self_mint {
                    return None;
                }

                // Reject tickers containing a null byte (reference parity and safety)
                // if v.tick.as_bytes().iter().any(|&b| b == 0) {
                //     return None;
                // }

                // Normalize unlimited self_mint tokens: when max==0, set an effective large cap for max/lim.
                let mut norm_max = v.max;
                let mut norm_lim = v.lim.unwrap_or(v.max);
                if v.self_mint && norm_max.is_zero() {
                    let cap = Fixed128::from(u64::MAX);
                    norm_max = cap;
                    if norm_lim.is_zero() {
                        norm_lim = cap;
                    }
                }

                self.token_actions.push(TokenAction::Deploy {
                    genesis: inc.genesis,
                    proto: DeployProtoDB {
                        tick: v.tick,
                        max: norm_max,
                        lim: norm_lim,
                        dec: v.dec,
                        self_mint: v.self_mint,
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
                // if log_this_tx {
                //     eprintln!(
                //         "[DEBUG_TX] token-mint tx={} tick={:?} amt={:?} owner_opret={} cursed={} unbound={}",
                //         inc.genesis.txid,
                //         proto.tick,
                //         proto.amt,
                //         inc.owner.is_op_return_hash(),
                //         inc.cursed_for_brc20,
                //         inc.unbound
                //     );
                // }
                self.token_actions.push(TokenAction::Mint {
                    owner: inc.owner,
                    proto,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
            }
            Brc4::Transfer { proto } => {
                // if log_this_tx {
                //     eprintln!(
                //         "[DEBUG_TX] token-transfer tx={} tick={:?} amt={:?}",
                //         inc.genesis.txid,
                //         proto.tick,
                //         proto.amt
                //     );
                // }
                self.token_actions.push(TokenAction::Transfer {
                    location: inc.location,
                    owner: inc.owner,
                    proto,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
                self.all_transfers.insert(inc.location, TransferProtoDB::from_proto(proto, height).ok()?);
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

    pub fn load_tokens_data(&mut self, _db: &DB, runtime: &RuntimeTokenState) -> anyhow::Result<()> {
        let (tickers, users) = self.fill_tickers_and_users();

        self.tokens = tickers
            .into_iter()
            .filter_map(|tick| runtime.tokens.get(&tick).cloned().map(|meta| (tick, meta)))
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

        self.token_accounts = keys.into_iter().filter_map(|key| runtime.balances.get(&key).cloned().map(|v| (key, v))).collect();

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
                    proto: MintProto { tick, .. },
                    ..
                } => {
                    tickers.insert((*tick).into());
                    users.insert((*owner, *tick));
                }
                TokenAction::Transfer {
                    owner,
                    proto: TransferProto { tick, .. },
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
                    let MintProto { tick, amt } = proto;
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

                    if *lim < amt {
                        continue;
                    }

                    // Safe-cap mint amount using remaining capacity (max is guaranteed > 0 after normalization)
                    let cap_left = *max - *supply;
                    if cap_left.is_zero() {
                        continue;
                    }
                    let amt = amt.min(cap_left);
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

                    let TransferProto { tick, amt } = proto;

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
