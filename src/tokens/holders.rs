use super::*;

#[derive(Eq, PartialEq, Clone, Ord, PartialOrd, Serialize, Deserialize, Debug)]
pub struct SortedByBalance(pub Fixed128, pub FullHash);

pub struct Holders {
    balances: parking_lot::RwLock<HashMap<OriginalTokenTick, BTreeSet<SortedByBalance>>>,
    stats: parking_lot::RwLock<HashMap<OriginalTokenTick, usize>>,
}

enum Action {
    Increase,
    Decrease,
}

impl Holders {
    pub fn init(db: &DB) -> Self {
        let holders = Self::build_from_db(db);

        let stats = holders.iter().map(|(tick, holders)| (*tick, holders.len())).collect();

        Self {
            balances: parking_lot::RwLock::new(holders),
            stats: parking_lot::RwLock::new(stats),
        }
    }

    fn build_from_db(db: &DB) -> HashMap<OriginalTokenTick, BTreeSet<SortedByBalance>> {
        HashMap::<OriginalTokenTick, _>::from_iter(
            db.address_token_to_balance
                .iter()
                .filter(|(_, v)| !v.balance.is_zero() || !v.transferable_balance.is_zero())
                .map(|(k, v)| (k.token, SortedByBalance(v.balance + v.transferable_balance, k.address)))
                .sorted_unstable_by_key(|(tick, _)| *tick)
                .chunk_by(|(tick, _)| *tick)
                .into_iter()
                .map(|(k, v)| (k, v.map(|(_, v)| v).collect::<BTreeSet<_>>())),
        )
    }

    /// Rebuild holders snapshot from RocksDB after events such as a reorg.
    pub fn rebuild_from_db(&self, db: &DB) {
        let holders = Self::build_from_db(db);
        let stats = holders.iter().map(|(tick, holders)| (*tick, holders.len())).collect();

        *self.balances.write() = holders;
        *self.stats.write() = stats;
    }

    pub fn get_holders(&self, tick: &OriginalTokenTick) -> Option<BTreeSet<SortedByBalance>> {
        self.balances.read().get(tick).cloned()
    }

    /// hack because i cant throw -amt cause of type
    pub fn decrease(&self, key: &AddressToken, prev_balance: &TokenBalance, amt: Fixed128) {
        self.change(key, prev_balance, amt, Action::Decrease);
    }

    /// returns `true` if new holder was created
    pub fn increase(&self, key: &AddressToken, prev_balance: &TokenBalance, amt: Fixed128) {
        self.change(key, prev_balance, amt, Action::Increase)
    }

    pub fn holders_by_tick(&self, tick: &OriginalTokenTick) -> Option<usize> {
        self.stats.read().get(tick).cloned()
    }

    pub fn stats(&self) -> HashMap<OriginalTokenTick, usize> {
        self.stats.read().clone()
    }

    fn change(&self, key: &AddressToken, acc: &TokenBalance, amt: Fixed128, action: Action) {
        // used to prevent footgun with balance (not to forget to add transferable)
        let old_balance = acc.balance + acc.transferable_balance;
        let mut balances = self.balances.write();

        let v = balances.entry(key.token).or_default();

        let existed = v.remove(&SortedByBalance(old_balance, key.address));

        match action {
            Action::Increase => {
                if !existed {
                    self.stats.write().entry(key.token).and_modify(|x| *x += 1).or_insert(1);
                }

                v.insert(SortedByBalance(old_balance + amt, key.address));
            }
            Action::Decrease => {
                let bal = old_balance - amt;
                if !bal.is_zero() {
                    v.insert(SortedByBalance(bal, key.address));
                } else {
                    self.stats.write().entry(key.token).and_modify(|x| *x -= 1);
                }
            }
        }
    }
}
