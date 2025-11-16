use super::*;

pub struct RuntimeTokenState {
    pub tokens: HashMap<LowerCaseTokenTick, TokenMeta>,
    pub balances: HashMap<AddressToken, TokenBalance>,
    pub valid_transfers: BTreeMap<Location, (FullHash, TransferProtoDB)>,
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

        let valid_transfers = db
            .address_location_to_transfer
            .iter()
            .map(|(k, v)| (k.location, (k.address, v)))
            .collect();

        Self {
            tokens,
            balances,
            valid_transfers,
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
            self.valid_transfers.remove(&loc);
        }

        for (addr_loc, proto_db) in transfers_to_write {
            let loc = addr_loc.location;
            let addr = addr_loc.address;
            self.valid_transfers.insert(loc, (addr, proto_db.clone()));
        }
    }
}
