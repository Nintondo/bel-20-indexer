use super::{config::*, error::SyncError, storage::BalanceV1};
use crate::{db::*, Server};
use anyhow::{ensure, Context};
use nintondo_holdings_protocol::{SnapshotCreated, SnapshotPage};
use rocksdb_wrapper::{Pebble, RocksSnapshot};
use std::{borrow::Cow, collections::HashMap, sync::Arc, time::Instant};

struct Session {
    snapshot: RocksSnapshot,
    created: Instant,
    touched: parking_lot::Mutex<Instant>,
}
impl Session {
    fn expired(&self) -> bool {
        self.created.elapsed() >= SNAPSHOT_LIFETIME || self.touched.lock().elapsed() >= SNAPSHOT_IDLE
    }
}

#[derive(Default)]
pub struct Snapshots(parking_lot::Mutex<HashMap<String, Arc<Session>>>);

pub fn decode_cursor(value: Option<&str>) -> anyhow::Result<Option<AddressToken>> {
    value
        .map(|value| {
            ensure!(value.len() == BALANCE_KEY_BYTES * 2, "Invalid snapshot cursor");
            let bytes = hex::decode(value)?;
            AddressToken::from_bytes(Cow::Owned(bytes))
        })
        .transpose()
}

impl Snapshots {
    pub fn expire(&self) {
        self.0.lock().retain(|_, session| !session.expired());
    }

    pub fn create(&self, server: &Server) -> anyhow::Result<SnapshotCreated> {
        // Never hold the session map while waiting for the writer gate.
        let (status, snapshot) = {
            let _gate = server.sync_gate.read();
            let status = super::status(server)?;
            if status.catching_up {
                return Err(SyncError::CatchingUp.into());
            }
            (status, server.db.storage.snapshot())
        };
        let mut sessions = self.0.lock();
        sessions.retain(|_, session| !session.expired());
        if sessions.len() >= SNAPSHOT_LIMIT {
            return Err(SyncError::Capacity.into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        sessions.insert(
            id.clone(),
            Arc::new(Session {
                snapshot,
                created: Instant::now(),
                touched: parking_lot::Mutex::new(Instant::now()),
            }),
        );
        Ok(SnapshotCreated { id, status })
    }

    pub fn close(&self, id: &str) {
        self.0.lock().remove(id);
    }

    pub fn page(&self, db: &DB, id: &str, after: Option<AddressToken>, limit: usize) -> anyhow::Result<SnapshotPage> {
        let session = {
            let sessions = self.0.lock();
            let session = sessions.get(id).filter(|session| !session.expired()).cloned().ok_or(SyncError::Expired)?;
            *session.touched.lock() = Instant::now();
            session
        };
        let snapshot = &session.snapshot;
        let mut items = Vec::new();
        let mut last_key = None;
        let mut more = false;
        for entry in snapshot.iter_after(&db.address_token_to_balance, after.as_ref()) {
            let (key, value) = entry?;
            if value.balance == crate::Fixed128::default() && value.transferable_balance == crate::Fixed128::default() {
                continue;
            }
            let Some(address) = snapshot.get(&db.fullhash_to_address, key.address)? else { continue };
            if items.len() == limit {
                more = true;
                break;
            }
            let token: LowerCaseTokenTick = key.token.into();
            let meta = snapshot.get(&db.token_to_meta, token)?.context("Snapshot token has no metadata")?;
            items.push(BalanceV1::new(address, key, &value, &meta).into_api());
            last_key = Some(hex::encode(AddressToken::get_bytes(&key)));
        }
        Ok(SnapshotPage {
            items,
            next_cursor: if more { last_key } else { None },
        })
    }
}
