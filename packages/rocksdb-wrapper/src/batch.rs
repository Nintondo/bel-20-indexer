use super::*;

/// Typed writes across column families, committed as one RocksDB transaction.
#[derive(Default)]
pub struct RocksBatch(WriteBatchWithTransaction<true>);

impl RocksBatch {
    pub fn set<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, key: impl Borrow<K::Inner>, value: impl Borrow<V::Inner>) {
        self.0.put_cf(&table.cf(), K::get_bytes(key.borrow()), V::get_bytes(value.borrow()));
    }

    pub fn extend<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, items: impl IntoIterator<Item = (impl Borrow<K::Inner>, impl Borrow<V::Inner>)>) {
        for (key, value) in items {
            self.set(table, key, value);
        }
    }

    pub fn remove<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, key: impl Borrow<K::Inner>) {
        self.0.delete_cf(&table.cf(), K::get_bytes(key.borrow()));
    }

    pub fn remove_batch<K: Pebble, V: Pebble>(&mut self, table: &RocksTable<K, V>, keys: impl IntoIterator<Item = impl Borrow<K::Inner>>) {
        for key in keys {
            self.remove(table, key);
        }
    }

    pub fn commit(self, db: &RocksDB) -> anyhow::Result<()> {
        db.db.write(self.0)?;
        Ok(())
    }
}
