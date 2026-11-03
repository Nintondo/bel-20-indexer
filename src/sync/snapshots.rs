use crate::{db::*, Server};
use anyhow::{ensure, Context};
use nintondo_holdings_protocol::{SnapshotCreated, SnapshotPage};
use rocksdb_wrapper::{Direction, IteratorMode, OptimisticTransactionDB, Pebble, SnapshotWithThreadMode, UsingSerde};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

type DbSnapshot<'a> = SnapshotWithThreadMode<'a, OptimisticTransactionDB>;
self_cell::self_cell! {
    struct SnapshotCell {
        owner: Arc<OptimisticTransactionDB>,
        #[covariant]
        dependent: DbSnapshot,
    }
}

struct Session {
    snapshot: SnapshotCell,
    touched: parking_lot::Mutex<Instant>,
}

#[derive(Default)]
pub struct Snapshots(parking_lot::Mutex<HashMap<String, Arc<Session>>>);

impl Snapshots {
    pub fn expire(&self) {
        self.0.lock().retain(|_, s| s.touched.lock().elapsed() < Duration::from_secs(900));
    }
    pub fn create(&self, server: &Server) -> anyhow::Result<SnapshotCreated> {
        let mut sessions = self.0.lock();
        sessions.retain(|_, s| s.touched.lock().elapsed() < Duration::from_secs(900));
        ensure!(sessions.len() < 4, "Too many active snapshots; close an existing snapshot");
        let _gate = server.sync_gate.read();
        let status = super::status(&server.db)?;
        let snapshot = SnapshotCell::new(server.db.last_block.db.db.clone(), |db| db.snapshot());
        let id = uuid::Uuid::new_v4().to_string();
        sessions.insert(
            id.clone(),
            Arc::new(Session {
                snapshot,
                touched: parking_lot::Mutex::new(Instant::now()),
            }),
        );
        Ok(SnapshotCreated { id, status })
    }

    pub fn close(&self, id: &str) {
        self.0.lock().remove(id);
    }

    pub fn page(&self, db: &DB, id: &str, cursor: Option<String>, limit: usize) -> anyhow::Result<SnapshotPage> {
        let session = {
            let mut sessions = self.0.lock();
            sessions.retain(|_, s| s.touched.lock().elapsed() < Duration::from_secs(900));
            let session = sessions.get(id).cloned().context("Snapshot expired or was closed")?;
            *session.touched.lock() = Instant::now();
            session
        };
        // The session owns its DB handle; the snapshot's lifetime cannot outlive it.
        let snapshot = session.snapshot.borrow_dependent();
        let from = cursor.as_deref().map(hex::decode).transpose()?;
        if let Some(key) = &from {
            ensure!(key.len() == 36, "Invalid snapshot cursor");
        }
        let mode = from.as_ref().map(|key| IteratorMode::From(key, Direction::Forward)).unwrap_or(IteratorMode::Start);
        let mut items = Vec::new();
        let mut last_key = None;
        let mut more = false;
        for entry in snapshot.iterator_cf(&db.address_token_to_balance.cf(), mode) {
            let (key_bytes, value_bytes) = entry?;
            if from.as_deref().is_some_and(|key| key_bytes.as_ref() <= key) {
                continue;
            }
            let key = AddressToken::from_bytes(Cow::Borrowed(&key_bytes))?;
            let value = UsingSerde::<TokenBalance>::from_bytes(Cow::Borrowed(&value_bytes))?;
            if value.balance == crate::Fixed128::default() && value.transferable_balance == crate::Fixed128::default() {
                continue;
            }
            let Some(address) = snapshot.get_cf(&db.fullhash_to_address.cf(), &key.address[..])? else {
                continue;
            };
            if items.len() == limit {
                more = true;
                break;
            }
            let token: LowerCaseTokenTick = key.token.into();
            let meta = snapshot
                .get_cf(&db.token_to_meta.cf(), LowerCaseTokenTick::get_bytes(&token))?
                .context("Snapshot token has no metadata")?;
            let meta = UsingSerde::<TokenMetaDB>::from_bytes(Cow::Owned(meta))?;
            let address = String::from_bytes(Cow::Owned(address))?;
            items.push(super::export::balance(address, key, value, meta));
            last_key = Some(hex::encode(&key_bytes));
        }
        Ok(SnapshotPage {
            items,
            next_cursor: if more { last_key } else { None },
        })
    }
}
