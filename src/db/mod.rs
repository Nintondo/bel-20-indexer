use super::*;

mod structs;
pub use structs::*;

use rocksdb_wrapper::{Pebble, RocksTable, WriteBatchWithTransaction};
use std::borrow::Borrow;

rocksdb_wrapper::generate_db_code! {
    token_to_meta: LowerCaseTokenTick => UsingSerde<TokenMetaDB>,
    address_location_to_transfer: AddressLocation => UsingSerde<TransferProtoDB>,
    address_token_to_balance: AddressToken => UsingSerde<TokenBalance>,
    address_token_to_history: AddressTokenIdDB => UsingSerde<HistoryValue>,
    block_info: u32 => BlockInfo,
    prevouts: UsingConsensus<OutPoint> => TxPrevout,
    outpoint_to_partials: UsingConsensus<OutPoint> => Partials,
    outpoint_to_inscription_offsets: UsingConsensus<OutPoint> => UsingSerde<BTreeMap<u64, bool>>,
    last_block: () => u32,
    last_history_id: () => u64,
    proof_of_history: u32 => UsingConsensus<sha256::Hash>,
    block_events: u32 => Vec<AddressTokenIdDB>,
    fullhash_to_address: FullHash => String,
    outpoint_to_event: UsingConsensus<OutPoint> => AddressTokenIdDB,
    token_id_to_event: TokenId => AddressTokenIdDB,
}

impl DB {
    pub fn load_token_accounts(&self, keys: Vec<AddressToken>) -> HashMap<AddressToken, TokenBalance> {
        self.address_token_to_balance.multi_get_kv(keys.iter(), false).into_iter().map(|(k, v)| (*k, v)).collect()
    }

    pub fn load_transfers(&self, keys: &HashSet<AddressOutPoint>) -> Vec<(Location, (FullHash, TransferProtoDB))> {
        keys.iter()
            .flat_map(|x| {
                let (from, to) = AddressLocation::search_with_offset(x.address, x.outpoint).into_inner();
                self.address_location_to_transfer.range(&from..=&to, false).collect_vec()
            })
            .map(|(key, value)| (key.location, (key.address, value)))
            .collect()
    }
}

pub struct DbBatch<'a> {
    pub db: &'a DB,
    pub batch: WriteBatchWithTransaction<true>,
}

impl<'a> DbBatch<'a> {
    pub fn new(db: &'a DB) -> Self {
        Self {
            db,
            batch: WriteBatchWithTransaction::<true>::default(),
        }
    }

    pub fn write(self) {
        // All tables share the same underlying RocksDB instance,
        // so using any table's db handle is fine.
        self.db.token_to_meta.db.db.write(self.batch).unwrap();
    }

    pub fn put<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, k: &K::Inner, v: &V::Inner) {
        let cf = table.cf();
        self.batch.put_cf(&cf, K::get_bytes(k), V::get_bytes(v));
    }
    
    #[allow(dead_code)]
    pub fn delete<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, k: &K::Inner) {
        let cf = table.cf();
        self.batch.delete_cf(&cf, K::get_bytes(k));
    }

    pub fn extend<K, V, I, BK, BV>(&mut self, table: &RocksTable<K, V>, kv: I)
    where
        K: Pebble,
        V: Pebble,
        I: IntoIterator<Item = (BK, BV)>,
        BK: Borrow<K::Inner>,
        BV: Borrow<V::Inner>,
    {
        let cf = table.cf();
        for (k, v) in kv {
            self.batch.put_cf(&cf, K::get_bytes(k.borrow()), V::get_bytes(v.borrow()));
        }
    }

    pub fn remove_batch<K, V, I, BK>(&mut self, table: &RocksTable<K, V>, keys: I)
    where
        K: Pebble,
        V: Pebble,
        I: IntoIterator<Item = BK>,
        BK: Borrow<K::Inner>,
    {
        let cf = table.cf();
        for k in keys {
            self.batch.delete_cf(&cf, K::get_bytes(k.borrow()));
        }
    }
}
