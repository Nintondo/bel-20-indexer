use std::cmp::Ordering;

use super::*;

#[derive(Clone)]
pub struct RocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl RocksDB {
    pub fn open_db(path: &str, tables: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = rocksdb::OptimisticTransactionDB::open_cf(&opts, path, tables).unwrap().arc();
        Self { db }
    }

    pub fn table<K: Pebble, V: Pebble>(&self, cf: impl ToString) -> RocksTable<K, V> {
        RocksTable {
            db: self.clone(),
            cf: cf.to_string(),
            __marker: PhantomData,
        }
    }
}

#[derive(Clone)]
pub struct RocksTable<K: Pebble, V: Pebble> {
    pub db: RocksDB,
    pub cf: String, // cf_handle() is just BTReeMap::get + RwLock::read + Arc::clone. Let's not fuck with lifetimes and pretend it's fine
    __marker: PhantomData<(K, V)>,
}

#[track_caller]
#[inline]
fn _panic(ident: &str, cf: &str, e: anyhow::Error) -> ! {
    panic!("Rocks {ident} '{cf}': {e:?}; bytes")
}

impl<K: Pebble, V: Pebble> RocksTable<K, V> {
    pub fn new(db: RocksDB, cf: String) -> Self {
        Self { db, cf, __marker: PhantomData }
    }

    pub fn table_info(&self) -> TableInfo {
        TableInfo::new::<K, V>()
    }

    pub fn cf<'a>(&'a self) -> Arc<rocksdb::BoundColumnFamily<'a>> {
        self.db.db.cf_handle(&self.cf).unwrap()
    }

    pub fn get(&self, k: impl Borrow<K::Inner>) -> Option<V::Inner> {
        self.db
            .db
            .get_cf(&self.cf(), K::get_bytes(k.borrow()))
            .unwrap()
            .map(|x| V::from_bytes(Cow::Owned(x)))
            .map(|x| x.unwrap_or_else(|e| _panic("get", &self.cf, e)))
    }

    pub fn multi_get<'a>(&'a self, keys: impl IntoIterator<Item = &'a K::Inner>) -> Vec<Option<V::Inner>> {
        let keys = keys.into_iter().map(|x| K::get_bytes(x)).collect::<Vec<_>>();
        self.db
            .db
            .batched_multi_get_cf(&self.cf(), keys.iter(), false)
            .into_iter()
            .map(|x| {
                x.unwrap()
                    .map(|x| V::from_bytes(Cow::Owned((*x).to_vec())).unwrap_or_else(|e| _panic("multi_get", &self.cf, e)))
            })
            .collect()
    }

    pub fn multi_get_kv<'a>(&'a self, keys: impl IntoIterator<Item = &'a K::Inner>, panic_if_not_exists: bool) -> Vec<(&'a K::Inner, V::Inner)> {
        let keys_bytes = keys.into_iter().map(|x| (x, K::get_bytes(x))).collect::<Vec<_>>();

        self.db
            .db
            .batched_multi_get_cf(&self.cf(), keys_bytes.iter().map(|x| &x.1), false)
            .into_iter()
            .map(|x| {
                x.unwrap()
                    .map(|x| V::from_bytes(Cow::Owned((*x).to_vec())).unwrap_or_else(|e| _panic("multi_get", &self.cf, e)))
            })
            .zip(keys_bytes.into_iter().map(|x| x.0))
            .filter_map(|(v, k)| {
                if panic_if_not_exists {
                    Some((k, v.unwrap_or_else(|| _panic("multi_get", &self.cf, anyhow::Error::msg("")))))
                } else {
                    Some((k, v?))
                }
            })
            .collect()
    }

    pub fn set(&self, k: impl Borrow<K::Inner>, v: impl Borrow<V::Inner>) {
        self.db.db.put_cf(&self.cf(), K::get_bytes(k.borrow()), V::get_bytes(v.borrow())).unwrap();
    }

    pub fn remove(&self, k: impl Borrow<K::Inner>) {
        self.db.db.delete_cf(&self.cf(), K::get_bytes(k.borrow())).unwrap();
    }

    pub fn iter(&self) -> impl Iterator<Item = (K::Inner, V::Inner)> + '_ {
        self.db
            .db
            .iterator_cf(&self.cf(), rocksdb::IteratorMode::Start)
            .flatten()
            .map(|(k, v)| (K::from_bytes(Cow::Owned(k.into_vec())), V::from_bytes(Cow::Owned(v.into_vec()))))
            .map(|(k, v)| (k.unwrap_or_else(|e| _panic("iter key", &self.cf, e)), v.unwrap_or_else(|e| _panic("iter val", &self.cf, e))))
    }

    pub fn range<'a>(&'a self, range: impl RangeBounds<&'a K::Inner>, reversed: bool) -> Box<dyn Iterator<Item = (K::Inner, V::Inner)> + 'a> {
        enum Position {
            Start,
            End,
        }
        enum BoundType {
            Included,
            Excluded,
            Unbounded,
        }

        let mut start = match range.start_bound() {
            Bound::Excluded(range) => (Position::Start, BoundType::Excluded, Some(K::get_bytes(range))),
            Bound::Included(range) => (Position::Start, BoundType::Included, Some(K::get_bytes(range))),
            Bound::Unbounded => (Position::Start, BoundType::Unbounded, None),
        };
        let mut end = match range.end_bound() {
            Bound::Excluded(range) => (Position::End, BoundType::Excluded, Some(K::get_bytes(range))),
            Bound::Included(range) => (Position::End, BoundType::Included, Some(K::get_bytes(range))),
            Bound::Unbounded => (Position::End, BoundType::Unbounded, None),
        };
        if reversed {
            std::mem::swap(&mut start, &mut end);
        }

        let (start_position, start_bound, start) = start;
        let (end_position, end_bound, end) = end;

        let (direction, mode) = if reversed {
            (rocksdb::Direction::Reverse, rocksdb::IteratorMode::End)
        } else {
            (rocksdb::Direction::Forward, rocksdb::IteratorMode::Start)
        };

        let x = self
            .db
            .db
            .iterator_cf(
                &self.cf(),
                if let Some(start) = start.as_ref() {
                    rocksdb::IteratorMode::From(start, direction)
                } else {
                    mode
                },
            )
            .flatten()
            .skip_while(move |(k, _)| matches!(start_bound, BoundType::Excluded) && **k == **start.as_ref().unwrap())
            .take_while(move |(k, _)| {
                let x = match end_bound {
                    BoundType::Unbounded => None,
                    _ => Some((**k).cmp(end.as_ref().unwrap())),
                };
                if let Some(x) = x {
                    if let Position::End = end_position {
                        if let BoundType::Included = end_bound {
                            x.is_le()
                        } else {
                            x.is_lt()
                        }
                    } else if let BoundType::Included = end_bound {
                        x.is_ge()
                    } else {
                        x.is_gt()
                    }
                } else {
                    true
                }
            })
            .map(move |(k, v)| (K::from_bytes(Cow::Owned(k.into_vec())), V::from_bytes(Cow::Owned(v.into_vec()))))
            .map(|(k, v)| {
                (
                    k.unwrap_or_else(|e| _panic("range key", &self.cf, e)),
                    v.unwrap_or_else(|e| _panic("range val", &self.cf, e)),
                )
            });

        Box::new(x)
    }

    pub fn retain(&self, f: impl Fn(K::Inner, V::Inner) -> bool) {
        let mut w = WriteBatchWithTransaction::<true>::default();
        let cf = self.cf();

        let iter = self
            .db
            .db
            .iterator_cf(&self.cf(), rocksdb::IteratorMode::Start)
            .flatten()
            .flat_map(|(k, v)| anyhow::Ok((K::from_bytes(Cow::Borrowed(&k))?, V::from_bytes(Cow::Owned(v.into_vec()))?, k)))
            .map(|(k, v, x)| (!(f)(k, v), x))
            .filter(|(b, _)| *b)
            .map(|(_, x)| x);
        for k in iter {
            w.delete_cf(&cf, k);
        }

        self.write(w);
    }

    pub fn flush(&self) {
        self.db.db.flush_cf(&self.cf()).unwrap();
    }

    pub fn write(&self, w: WriteBatchWithTransaction<true>) {
        self.db.db.write(w).unwrap();
    }

    pub fn extend(&self, kv: impl IntoIterator<Item = (impl Borrow<K::Inner>, impl Borrow<V::Inner>)>) {
        let mut w = WriteBatchWithTransaction::<true>::default();
        let cf = self.cf();
        for (k, v) in kv {
            w.put_cf(&cf, K::get_bytes(k.borrow()), V::get_bytes(v.borrow()));
        }
        self.write(w);
    }

    pub fn remove_batch(&self, k: impl IntoIterator<Item = impl Borrow<K::Inner>>) {
        let mut w = WriteBatchWithTransaction::<true>::default();
        let cf = self.cf();
        for k in k {
            w.delete_cf(&cf, K::get_bytes(k.borrow()));
        }
        self.write(w);
    }
}
