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
}

