use super::*;
use rocksdb::{Direction, IteratorMode, OptimisticTransactionDB, SnapshotWithThreadMode};

type Snapshot<'a> = SnapshotWithThreadMode<'a, OptimisticTransactionDB>;
self_cell::self_cell! {
    struct SnapshotCell {
        owner: Arc<OptimisticTransactionDB>,
        #[covariant]
        dependent: Snapshot,
    }
}

/// Owns the database reference and provides fallible, typed snapshot reads.
pub struct RocksSnapshot(SnapshotCell);

impl RocksDB {
    pub fn snapshot(&self) -> RocksSnapshot {
        RocksSnapshot(SnapshotCell::new(self.db.clone(), |db| db.snapshot()))
    }

    pub fn delete_range<K: Pebble, V: Pebble>(&self, table: &RocksTable<K, V>, from: &K::Inner, to: &K::Inner) -> anyhow::Result<()> {
        self.db.delete_range_cf(&table.cf(), K::get_bytes(from), K::get_bytes(to))?;
        Ok(())
    }
}

impl<K: Pebble, V: Pebble> RocksTable<K, V> {
    pub fn keys_between(&self, from: &K::Inner, to: &K::Inner) -> anyhow::Result<Vec<K::Inner>> {
        let from = K::get_bytes(from);
        let to = K::get_bytes(to);
        let mut keys = Vec::new();
        for entry in self.db.db.iterator_cf(&self.cf(), IteratorMode::From(&from, Direction::Forward)) {
            let (key, _) = entry?;
            if key.as_ref() >= to.as_ref() {
                break;
            }
            keys.push(K::from_bytes(Cow::Owned(key.into_vec()))?);
        }
        Ok(keys)
    }

    pub fn try_get(&self, key: impl Borrow<K::Inner>) -> anyhow::Result<Option<V::Inner>> {
        self.db
            .db
            .get_cf(&self.cf(), K::get_bytes(key.borrow()))?
            .map(|bytes| V::from_bytes(Cow::Owned(bytes)))
            .transpose()
    }
}

impl RocksSnapshot {
    pub fn get<K: Pebble, V: Pebble>(&self, table: &RocksTable<K, V>, key: impl Borrow<K::Inner>) -> anyhow::Result<Option<V::Inner>> {
        self.0
            .borrow_dependent()
            .get_cf(&table.cf(), K::get_bytes(key.borrow()))?
            .map(|bytes| V::from_bytes(Cow::Owned(bytes)))
            .transpose()
    }

    pub fn iter_after<'a, K: Pebble, V: Pebble>(&'a self, table: &RocksTable<K, V>, after: Option<&K::Inner>) -> impl Iterator<Item = anyhow::Result<(K::Inner, V::Inner)>> + 'a {
        let after = after.map(|key| K::get_bytes(key).into_owned());
        let mode = after.as_ref().map(|key| IteratorMode::From(key, Direction::Forward)).unwrap_or(IteratorMode::Start);
        self.0.borrow_dependent().iterator_cf(&table.cf(), mode).filter_map(move |entry| match entry {
            Ok((key, _)) if after.as_deref().is_some_and(|start| key.as_ref() <= start) => None,
            Ok((key, value)) => Some(K::from_bytes(Cow::Owned(key.into_vec())).and_then(|key| V::from_bytes(Cow::Owned(value.into_vec())).map(|value| (key, value)))),
            Err(error) => Some(Err(error.into())),
        })
    }
}
