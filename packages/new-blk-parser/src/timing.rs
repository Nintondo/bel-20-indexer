use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Timing metrics for block reading operations
pub struct BlockReadMetrics {
    pub block_read_time_ns: AtomicU64,
    pub block_read_count: AtomicU64,
    pub xor_time_ns: AtomicU64,
    pub xor_count: AtomicU64,
    pub header_time_ns: AtomicU64,
    pub header_count: AtomicU64,
    pub tx_decode_time_ns: AtomicU64,
    pub tx_decode_count: AtomicU64,
    pub script_eval_time_ns: AtomicU64,
    pub script_eval_count: AtomicU64,
    pub hash_time_ns: AtomicU64,
    pub hash_count: AtomicU64,
    pub varint_time_ns: AtomicU64,
    pub varint_count: AtomicU64,
    pub auxpow_time_ns: AtomicU64,
    pub auxpow_count: AtomicU64,
}

impl BlockReadMetrics {
    pub const fn new() -> Self {
        Self {
            block_read_time_ns: AtomicU64::new(0),
            block_read_count: AtomicU64::new(0),
            xor_time_ns: AtomicU64::new(0),
            xor_count: AtomicU64::new(0),
            header_time_ns: AtomicU64::new(0),
            header_count: AtomicU64::new(0),
            tx_decode_time_ns: AtomicU64::new(0),
            tx_decode_count: AtomicU64::new(0),
            script_eval_time_ns: AtomicU64::new(0),
            script_eval_count: AtomicU64::new(0),
            hash_time_ns: AtomicU64::new(0),
            hash_count: AtomicU64::new(0),
            varint_time_ns: AtomicU64::new(0),
            varint_count: AtomicU64::new(0),
            auxpow_time_ns: AtomicU64::new(0),
            auxpow_count: AtomicU64::new(0),
        }
    }

    pub fn record(&self, duration: Duration) {
        self.block_read_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_read_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_xor(&self, duration: Duration) {
        self.xor_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.xor_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_header(&self, duration: Duration) {
        self.header_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.header_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tx_decode(&self, duration: Duration, count: u64) {
        self.tx_decode_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.tx_decode_count.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_script_eval(&self, duration: Duration) {
        self.script_eval_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.script_eval_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_hash(&self, duration: Duration) {
        self.hash_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.hash_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_varint(&self, duration: Duration) {
        self.varint_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.varint_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_auxpow(&self, duration: Duration) {
        self.auxpow_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.auxpow_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Consume and reset the timing values
    pub fn take(&self) -> (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) {
        let time_ns = self.block_read_time_ns.swap(0, Ordering::Relaxed);
        let count = self.block_read_count.swap(0, Ordering::Relaxed);
        let xor_ns = self.xor_time_ns.swap(0, Ordering::Relaxed);
        let xor_count = self.xor_count.swap(0, Ordering::Relaxed);
        let header_ns = self.header_time_ns.swap(0, Ordering::Relaxed);
        let header_count = self.header_count.swap(0, Ordering::Relaxed);
        let tx_ns = self.tx_decode_time_ns.swap(0, Ordering::Relaxed);
        let tx_count = self.tx_decode_count.swap(0, Ordering::Relaxed);
        let script_ns = self.script_eval_time_ns.swap(0, Ordering::Relaxed);
        let script_count = self.script_eval_count.swap(0, Ordering::Relaxed);
        let hash_ns = self.hash_time_ns.swap(0, Ordering::Relaxed);
        let hash_count = self.hash_count.swap(0, Ordering::Relaxed);
        let varint_ns = self.varint_time_ns.swap(0, Ordering::Relaxed);
        let varint_count = self.varint_count.swap(0, Ordering::Relaxed);
        let auxpow_ns = self.auxpow_time_ns.swap(0, Ordering::Relaxed);
        let auxpow_count = self.auxpow_count.swap(0, Ordering::Relaxed);
        (
            time_ns,
            count,
            xor_ns,
            xor_count,
            header_ns,
            header_count,
            tx_ns,
            tx_count,
            script_ns,
            script_count,
            hash_ns,
            hash_count,
            varint_ns,
            varint_count,
            auxpow_ns,
            auxpow_count,
        )
    }
}

/// Global block read metrics instance
pub static BLOCK_READ_METRICS: BlockReadMetrics = BlockReadMetrics::new();
