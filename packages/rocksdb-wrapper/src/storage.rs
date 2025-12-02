use std::cmp::Ordering;

use super::*;

#[derive(Clone)]
pub struct RocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl RocksDB {
    pub fn open_db(path: &str, tables: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        // Collect CF names up front so we can attach per-CF options.
        let table_names: Vec<String> = tables.into_iter().map(|t| t.as_ref().to_string()).collect();

        // DB-wide options (WAL, parallelism, etc.).
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        db_opts.increase_parallelism(16);
        db_opts.set_max_background_jobs(8);
        db_opts.set_max_open_files(-1); // keep all files open, if OS limits allow

        db_opts.set_max_subcompactions(4);

        // Baseline CF options shared by most column families.
        let mut base_cf_opts = rocksdb::Options::default();

        // Optimize write path for ~4 GiB of RocksDB memory.
        base_cf_opts.optimize_level_style_compaction(4 * 1024 * 1024 * 1024);

        // Bigger memtables and dynamic level sizes.
        base_cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128 MB
        base_cf_opts.set_max_write_buffer_number(8);
        base_cf_opts.set_min_write_buffer_number_to_merge(2);  // Flush 2 at a time if needed

        // Larger SSTables → fewer compactions.
        base_cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128 MiB files

        // Smoother fsync.
        base_cf_opts.set_bytes_per_sync(2 * 1024 * 1024);
        base_cf_opts.set_wal_bytes_per_sync(2 * 1024 * 1024);

        // Shared block cache and block-based options.
        let block_cache = rocksdb::Cache::new_lru_cache(4 * 1024 * 1024 * 1024);

        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_cache(&block_cache);
        block_opts.set_bloom_filter(10.0, false);
        block_opts.set_partition_filters(true);
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);        
        block_opts.set_cache_index_and_filter_blocks(true);
        block_opts.set_data_block_index_type(rocksdb::DataBlockIndexType::BinaryAndHash);

        base_cf_opts.set_block_based_table_factory(&block_opts);


        // Hot CFs (prevouts, outpoint_to_inscription_offsets) get lighter compression
        // and prefix-based bloom filters keyed by the 32-byte txid prefix, but reuse
        // the same shared block cache.
        let block_cache_hot = rocksdb::Cache::new_lru_cache(8 * 1024 * 1024 * 1024);

        let mut hot_cf_opts = base_cf_opts.clone();
        hot_cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(32));
        hot_cf_opts.set_memtable_prefix_bloom_ratio(0.2);

        let mut hot_block_opts = rocksdb::BlockBasedOptions::default();
        hot_block_opts.set_block_cache(&block_cache_hot);
        hot_block_opts.set_bloom_filter(10.0, false);
        hot_block_opts.set_partition_filters(true);
        hot_block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);        
        hot_block_opts.set_cache_index_and_filter_blocks(true);
        hot_block_opts.set_data_block_index_type(rocksdb::DataBlockIndexType::BinaryAndHash);
        hot_block_opts.set_whole_key_filtering(true);

        hot_cf_opts.set_block_based_table_factory(&hot_block_opts);


        let cf_descriptors: Vec<rocksdb::ColumnFamilyDescriptor> = table_names
            .into_iter()
            .map(|name| {
                let opts = match name.as_str() {
                    "prevouts" | "outpoint_to_inscription_offsets" => hot_cf_opts.clone(),
                    _ => base_cf_opts.clone(),
                };
                rocksdb::ColumnFamilyDescriptor::new(name, opts)
            })
            .collect();

        let db = rocksdb::OptimisticTransactionDB::open_cf_descriptors(&db_opts, path, cf_descriptors)
            .unwrap()
            .arc();
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
                    .map(|x| V::from_bytes(Cow::Borrowed(x.as_ref())).unwrap_or_else(|e| _panic("multi_get", &self.cf, e)))
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
                    .map(|x| V::from_bytes(Cow::Borrowed(x.as_ref())).unwrap_or_else(|e| _panic("multi_get", &self.cf, e)))
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
