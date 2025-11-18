use super::*;

pub struct RuntimeTokenState {
    pub tokens: HashMap<LowerCaseTokenTick, TokenMeta>,
    pub balances: HashMap<AddressToken, TokenBalance>,
    pub valid_transfers: BTreeMap<Location, (FullHash, TransferProtoDB)>,
    // Secondary index: (address, outpoint) -> all locations with active transfers
    pub transfers_by_outpoint: HashMap<AddressOutPoint, Vec<Location>>,
}

impl RuntimeTokenState {
    pub fn from_db(db: &DB) -> Self {
        let tokens = db
            .token_to_meta
            .iter()
            .map(|(k, v)| (k, TokenMeta::from(v)))
            .collect();

        let balances = db
            .address_token_to_balance
            .iter()
            .map(|(k, v)| (k, v))
            .collect();

        // Build both primary map and secondary index in a single pass.
        let mut valid_transfers = BTreeMap::<Location, (FullHash, TransferProtoDB)>::new();
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

    pub fn apply_tokens_delta(
        &mut self,
        metas: &[(LowerCaseTokenTick, TokenMetaDB)],
        balances: &[(AddressToken, TokenBalance)],
        transfers_to_write: &[(AddressLocation, TransferProtoDB)],
        transfers_to_remove: &[AddressLocation],
    ) {
        for (tick, meta_db) in metas {
            self.tokens.insert(tick.clone(), TokenMeta::from(meta_db.clone()));
        }

        for (key, balance) in balances {
            self.balances.insert(*key, balance.clone());
        }

        for addr_loc in transfers_to_remove {
            let loc = addr_loc.location;
            let idx_key = AddressOutPoint {
                address: addr_loc.address,
                outpoint: loc.outpoint,
            };

            // Remove from primary map
            self.valid_transfers.remove(&loc);

            // Remove from secondary index
            if let Some(locs) = self.transfers_by_outpoint.get_mut(&idx_key) {
                if let Some(pos) = locs.iter().position(|x| *x == loc) {
                    locs.swap_remove(pos);
                }
                if locs.is_empty() {
                    self.transfers_by_outpoint.remove(&idx_key);
                }
            }
        }

        for (addr_loc, proto_db) in transfers_to_write {
            let loc = addr_loc.location;
            let addr = addr_loc.address;
            let idx_key = AddressOutPoint {
                address: addr,
                outpoint: loc.outpoint,
            };

            // Insert into primary map
            self.valid_transfers.insert(loc, (addr, proto_db.clone()));

            // Insert into secondary index
            self.transfers_by_outpoint.entry(idx_key).or_default().push(loc);
        }
    }
}
