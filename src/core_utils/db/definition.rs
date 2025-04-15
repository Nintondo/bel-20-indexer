use super::*;

pub trait RocksDbTablesDef: Sized {
    const TABLES: &[&str];
    const VERSION: usize;

    fn table_info(&self, cf: &str) -> TableInfo;
    fn make_tables(db: RocksDB) -> Self;

    fn open(path: &str) -> anyhow::Result<Self> {
        let db = RocksDB::open_db(
            path,
            [
                &[internal::TABLE_INFO_CF, internal::DB_INFO_CF],
                Self::TABLES,
            ]
            .into_iter()
            .flatten(),
        );

        let mut tables = Self::make_tables(db.clone());

        let db_info = db.table::<(), UsingSerde<DbInfo>>(internal::DB_INFO_CF);
        if let Some(db_info) = db_info.get(()) {
            if db_info.version > Self::VERSION {
                bail!(
                    "Version of DB '{}' is not supported: {}",
                    std::any::type_name::<Self>(),
                    db_info.version
                );
            }
            if db_info.version < Self::VERSION {
                warn!(
                    "Db '{}' version is outdated. Trying to upgrade from {} to {}",
                    std::any::type_name::<Self>(),
                    db_info.version,
                    Self::VERSION
                );
                tables.migrate(db_info.version)?;
            }
        }

        db_info.set(
            (),
            DbInfo {
                version: Self::VERSION,
            },
        );

        Ok(tables)
    }

    fn migrate(&mut self, version: usize) -> anyhow::Result<()> {
        let _ = version;
        bail!("Migration from version {version} is not supported")
    }
}
