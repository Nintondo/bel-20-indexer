use super::*;
use crate::inscriptions::ProcessedData;

/// Global in-memory token state built from RocksDB and incrementally
/// updated as new blocks are indexed.
pub struct RuntimeTokenState {
    pub tokens: HashMap<LowerCaseTokenTick, TokenMeta>,
    pub balances: HashMap<AddressToken, TokenBalance>,
    pub valid_transfers: HashMap<Location, (FullHash, TransferProtoDB)>,
    // Secondary index: (address, outpoint) -> all locations with active transfers
    pub transfers_by_outpoint: HashMap<AddressOutPoint, Vec<Location>>,
}

impl RuntimeTokenState {
    pub fn from_db(db: &DB) -> Self {
        let tokens = db.token_to_meta.iter().map(|(k, v)| (k, TokenMeta::from(v))).collect();

        let balances = db.address_token_to_balance.iter().collect();

        // Build both primary map and secondary index in a single pass.
        let mut valid_transfers = HashMap::<Location, (FullHash, TransferProtoDB)>::new();
        let mut transfers_by_outpoint = HashMap::<AddressOutPoint, Vec<Location>>::new();

        for (addr_loc, proto) in db.address_location_to_transfer.iter() {
            let loc = addr_loc.location;
            let addr = addr_loc.address;

            valid_transfers.insert(loc, (addr, proto.clone()));

            let key = AddressOutPoint {
                address: addr,
                outpoint: loc.outpoint,
            };

            transfers_by_outpoint.entry(key).or_default().push(loc);
        }

        Self {
            tokens,
            balances,
            valid_transfers,
            transfers_by_outpoint,
        }
    }
}

/// Per-block view over [`RuntimeTokenState`].
///
/// This struct collects token actions discovered while parsing a block,
/// applies them directly to the global runtime state, and produces the
/// minimal RocksDB delta (`ProcessedData::Tokens`) needed to persist the
/// changes and support reorgs.
pub struct BlockTokenState<'a> {
    pub rt: &'a mut RuntimeTokenState,
    pub server: Arc<Server>,

    // Per-block actions collected from inscriptions / transfers.
    pub token_actions: Vec<TokenAction>,

    // In-block transfer prototypes (created by Transfer inscriptions).
    pub all_transfers: HashMap<Location, TransferProtoDB>,

    // Snapshot of transfers that were valid before this block, restricted
    // to prevouts of this block. This map is mutated to represent the
    // post-block valid transfer set for those locations.
    pub valid_transfers: HashMap<Location, (FullHash, TransferProtoDB)>,

    // Pre-block valid transfers for this block's prevouts, captured as
    // AddressLocation keys so we can generate `transfers_to_remove` for DB.
    pub transfers_to_remove: Vec<AddressLocation>,

    // Scratch sets to record which metas / balances changed so we can
    // build DB deltas without rescanning the whole runtime.
    touched_ticks: hashbrown::HashSet<LowerCaseTokenTick>,
    touched_accounts: hashbrown::HashSet<AddressToken>,
}

impl<'a> BlockTokenState<'a> {
    pub fn new(rt: &'a mut RuntimeTokenState, server: Arc<Server>, prevouts: &hashbrown::HashMap<OutPoint, TxPrevout>) -> Self {
        // Build a pre-block snapshot of transfers reachable via this
        // block's prevouts using the secondary index.
        let mut valid_transfers = HashMap::<Location, (FullHash, TransferProtoDB)>::new();
        let mut transfers_to_remove = Vec::new();

        for (outpoint, prev) in prevouts {
            let key = AddressOutPoint {
                address: prev.script_hash,
                outpoint: *outpoint,
            };
            if let Some(locs) = rt.transfers_by_outpoint.get(&key) {
                for loc in locs {
                    if let Some((addr, proto)) = rt.valid_transfers.get(loc) {
                        valid_transfers.insert(*loc, (*addr, proto.clone()));
                    }
                }
            }
        }

        for (loc, (addr, _)) in &valid_transfers {
            transfers_to_remove.push(AddressLocation {
                address: *addr,
                location: *loc,
            });
        }

        // Pre-populate all_transfers from valid_transfers so that when a
        // TokenAction::Transfer tries to "spend" a pre-existing transfer,
        // it will find it. New transfers created in this block will also
        // be inserted into all_transfers via register_transfer / parse_token_action.
        let all_transfers: HashMap<Location, TransferProtoDB> = valid_transfers
            .iter()
            .map(|(loc, (_, proto))| (*loc, proto.clone()))
            .collect();

        Self {
            rt,
            server,
            token_actions: Vec::new(),
            all_transfers,
            valid_transfers,
            transfers_to_remove,
            touched_ticks: hashbrown::HashSet::new(),
            touched_accounts: hashbrown::HashSet::new(),
        }
    }

    /// Record a parsed inscription as a token action.
    pub fn push_action(&mut self, action: TokenAction) {
        self.token_actions.push(action);
    }

    /// Register a newly created transfer prototype for the given location.
    pub fn register_transfer(&mut self, location: Location, proto: TransferProtoDB) {
        self.all_transfers.insert(location, proto);
    }

    fn sync_runtime_transfers(&mut self) {
        // First remove all pre-block transfers for this block's prevouts.
        for addr_loc in &self.transfers_to_remove {
            let loc = addr_loc.location;
            let idx_key = AddressOutPoint {
                address: addr_loc.address,
                outpoint: loc.outpoint,
            };

            self.rt.valid_transfers.remove(&loc);

            if let Some(locs) = self.rt.transfers_by_outpoint.get_mut(&idx_key) {
                if let Some(pos) = locs.iter().position(|x| *x == loc) {
                    locs.swap_remove(pos);
                }
                if locs.is_empty() {
                    self.rt.transfers_by_outpoint.remove(&idx_key);
                }
            }
        }

        // Then insert the post-block snapshot for all locations touched
        // in this block.
        for (loc, (address, proto)) in &self.valid_transfers {
            self.rt.valid_transfers.insert(*loc, (*address, proto.clone()));

            let idx_key = AddressOutPoint {
                address: *address,
                outpoint: loc.outpoint,
            };
            self.rt.transfers_by_outpoint.entry(idx_key).or_default().push(*loc);
        }
    }

    /// Apply all collected actions to the runtime state and build history
    /// + RocksDB deltas for this block.
    pub fn finish(
        &mut self,
        holders: &Holders,
        _height: u32,
        _created: u32,
    ) -> (Vec<HistoryTokenAction>, ProcessedData) {
        let mut history = Vec::<HistoryTokenAction>::new();

        for action in self.token_actions.drain(..) {
            match action {
                TokenAction::Deploy { genesis, proto, owner } => {
                    let DeployProtoDB { tick, max, lim, dec, .. } = proto.clone();
                    let tick_lc: LowerCaseTokenTick = tick.into();

                    let mut deployed = false;
                    match self.rt.tokens.entry(tick_lc.clone()) {
                        std::collections::hash_map::Entry::Vacant(e) => {
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
                            deployed = true;
                        }
                        std::collections::hash_map::Entry::Occupied(_) => {
                            // Duplicate deploy is ignored, as before.
                        }
                    }

                    if deployed {
                        self.touched_ticks.insert(tick_lc);
                    }
                }
                TokenAction::Mint { owner, proto, txid, vout } => {
                    let MintProto { tick: tick_orig, amt } = proto;
                    let tick_lc: LowerCaseTokenTick = tick_orig.into();
                    let mut did_change = false;

                    // Declare key outside the block so it's accessible for touched_accounts
                    let mut key: Option<AddressToken> = None;

                    {
                        let Some(token) = self.rt.tokens.get_mut(&tick_lc) else {
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

                        let cap_left = *max - *supply;
                        if cap_left.is_zero() {
                            continue;
                        }
                        let amt = amt.min(cap_left);
                        *supply += amt;

                        *transactions += 1;

                        // Use canonical tick from token.proto, not tick_orig from inscription
                        let balance_key = AddressToken { address: owner, token: *tick };
                        key = Some(balance_key);

                        let entry = self.rt.balances.entry(balance_key).or_default();
                        holders.increase(&balance_key, entry, amt);
                        entry.balance += amt;
                        *mint_count += 1;

                        history.push(HistoryTokenAction::Mint {
                            tick: *tick,
                            amt,
                            recipient: balance_key.address,
                            txid,
                            vout,
                        });

                        did_change = true;
                    }

                    if did_change {
                        self.touched_ticks.insert(tick_lc);
                        if let Some(k) = key {
                            self.touched_accounts.insert(k);
                        }
                    }
                }
                TokenAction::Transfer {
                    owner,
                    location,
                    proto,
                    txid,
                    vout,
                } => {
                    let Some(mut data) = self.all_transfers.remove(&location) else {
                        // Transfer already spent or invalid; skip.
                        continue;
                    };

                    let TransferProto { tick: tick_orig, amt } = proto;
                    let tick_lc: LowerCaseTokenTick = tick_orig.into();
                    let mut did_change = false;

                    // Declare key outside the block so it's accessible for touched_accounts
                    let mut key: Option<AddressToken> = None;

                    {
                        let Some(token) = self.rt.tokens.get_mut(&tick_lc) else {
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
                            continue;
                        }

                        // Use canonical tick from token.proto, not tick_orig from inscription
                        let balance_key = AddressToken { address: owner, token: *tick };
                        key = Some(balance_key);

                        let Some(account) = self.rt.balances.get_mut(&balance_key) else {
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
                            recipient: balance_key.address,
                            txid,
                            vout,
                        });

                        self.valid_transfers.insert(location, (balance_key.address, data));
                        *transfer_count += 1;
                        *transactions += 1;

                        did_change = true;
                    }

                    if did_change {
                        self.touched_ticks.insert(tick_lc);
                        if let Some(k) = key {
                            self.touched_accounts.insert(k);
                        }
                    }
                }
                TokenAction::Transferred {
                    transfer_location,
                    recipient,
                    txid,
                    vout,
                } => {
                    let Some((sender, TransferProtoDB { tick, amt, .. })) = self.valid_transfers.remove(&transfer_location) else {
                        // Transfer already spent; skip.
                        continue;
                    };

                    let tick_lc: LowerCaseTokenTick = tick.into();
                    let mut touched_recipient: Option<AddressToken> = None;
                    let mut old_key: Option<AddressToken> = None;

                    {
                        let Some(token) = self.rt.tokens.get_mut(&tick_lc) else {
                            // Token doesn't exist (shouldn't happen for valid transfers).
                            continue;
                        };

                        let DeployProtoDB { transactions, tick, .. } = &mut token.proto;

                        // Create old_key AFTER getting canonical tick from token.proto
                        let sender_key = AddressToken { address: sender, token: *tick };
                        old_key = Some(sender_key);

                        let Some(old_account) = self.rt.balances.get_mut(&sender_key) else {
                            // Sender account missing - this can happen if:
                            // 1. The transfer was created in a prior block but sender's balance
                            //    was zeroed out and removed, OR
                            // 2. Data inconsistency from a reorg.
                            // Skip this transfer to maintain forward progress.
                            continue;
                        };

                        if old_account.transfers_count == 0 || old_account.transferable_balance < amt {
                            // Keep the same invariant as before; this is a logic error.
                            panic!("Invalid transfer sender balance");
                        }

                        holders.decrease(&sender_key, old_account, amt);
                        old_account.transfers_count -= 1;
                        old_account.transferable_balance -= amt;
                        *transactions += 1;

                        if !recipient.is_op_return_hash() {
                            let recipient_key = AddressToken { address: recipient, token: *tick };
                            let recipient_account = self.rt.balances.entry(recipient_key).or_default();

                            holders.increase(&recipient_key, recipient_account, amt);
                            recipient_account.balance += amt;

                            touched_recipient = Some(recipient_key);
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

                    self.touched_ticks.insert(tick_lc);
                    if let Some(k) = old_key {
                        self.touched_accounts.insert(k);
                    }
                    if let Some(rec_key) = touched_recipient {
                        self.touched_accounts.insert(rec_key);
                    }
                }
            }
        }

        // Bring runtime's transfer maps in sync for all locations affected
        // by this block.
        self.sync_runtime_transfers();

        // Build DB deltas from touched ticks/accounts.
        let metas = self
            .touched_ticks
            .iter()
            .filter_map(|tick| self.rt.tokens.get(tick).cloned().map(|meta| (tick.clone(), TokenMetaDB::from(meta))))
            .collect::<Vec<_>>();

        let balances = self
            .touched_accounts
            .iter()
            .filter_map(|key| self.rt.balances.get(key).cloned().map(|bal| (*key, bal)))
            .collect::<Vec<_>>();

        let transfers_to_write = self
            .valid_transfers
            .iter()
            .map(|(location, (address, proto))| (AddressLocation { address: *address, location: *location }, proto.clone()))
            .collect::<Vec<_>>();

        let transfers_to_remove = self.transfers_to_remove.clone();

        let tokens_pd = ProcessedData::Tokens {
            metas,
            balances,
            transfers_to_write,
            transfers_to_remove,
        };

        (history, tokens_pd)
    }
}
