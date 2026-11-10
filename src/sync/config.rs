use anyhow::ensure;
use std::time::Duration;

pub const SNAPSHOT_IDLE: Duration = Duration::from_secs(900);
pub const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(3600);
pub const SNAPSHOT_LIMIT: usize = 4;
pub const PAGE_DEFAULT: usize = 500;
pub const PAGE_MAX: usize = 1000;
pub const BLOCK_DEFAULT: usize = 50;
pub const BLOCK_MAX: usize = 100;
pub const REPLAY_PAGE_BYTES: usize = 16 * 1024 * 1024;
pub const REPLAY_RETENTION: u32 = 10_000;
pub const PAGE_SCAN_MAX: usize = 5000;
pub const EXPORT_WORKERS: usize = 4;
pub const EVENT_CONNECTIONS: usize = 16;
pub const BALANCE_KEY_BYTES: usize = 36;

pub struct SyncConfig {
    token: Option<String>,
    pub retention: u32,
}

impl SyncConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("SYNC_TOKEN").ok().filter(|token| !token.is_empty());
        if let Some(token) = &token {
            ensure!(
                token.len() >= 32 && token.bytes().all(|b| b.is_ascii_graphic()),
                "SYNC_TOKEN must contain at least 32 printable ASCII characters"
            );
        }
        let retention = REPLAY_RETENTION;
        Ok(Self { token, retention })
    }

    pub fn enabled(&self) -> bool {
        self.token.is_some()
    }

    pub fn authorized(&self, supplied: Option<&str>) -> bool {
        use subtle::ConstantTimeEq;
        self.token
            .as_deref()
            .zip(supplied)
            .is_some_and(|(expected, supplied)| bool::from(expected.as_bytes().ct_eq(supplied.as_bytes())))
    }
}
