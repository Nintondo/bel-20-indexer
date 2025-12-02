use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Global timing metrics for indexing performance analysis
pub struct IndexingMetrics {
    // Block reading from blk files (collected from parser crate)
    pub block_read_time_ns: AtomicU64,
    pub block_read_count: AtomicU64,
    pub block_read_xor_time_ns: AtomicU64,
    pub block_read_xor_count: AtomicU64,
    pub block_read_header_time_ns: AtomicU64,
    pub block_read_header_count: AtomicU64,
    pub block_read_tx_time_ns: AtomicU64,
    pub block_read_tx_count: AtomicU64,
    pub block_read_script_time_ns: AtomicU64,
    pub block_read_script_count: AtomicU64,
    pub block_read_hash_time_ns: AtomicU64,
    pub block_read_hash_count: AtomicU64,
    pub block_read_varint_time_ns: AtomicU64,
    pub block_read_varint_count: AtomicU64,
    pub block_read_auxpow_time_ns: AtomicU64,
    pub block_read_auxpow_count: AtomicU64,

    // Block parsing (inscriptions, tokens)
    pub block_parse_time_ns: AtomicU64,
    pub block_parse_count: AtomicU64,
    pub block_parse_partials_time_ns: AtomicU64,
    pub block_parse_partials_count: AtomicU64,
    pub block_parse_offsets_time_ns: AtomicU64,
    pub block_parse_offsets_count: AtomicU64,
    pub block_parse_tx_time_ns: AtomicU64,
    pub block_parse_tx_count: AtomicU64,
    pub block_parse_tx_offsets_time_ns: AtomicU64,
    pub block_parse_tx_offsets_count: AtomicU64,
    pub block_parse_tx_inputs_time_ns: AtomicU64,
    pub block_parse_tx_inputs_count: AtomicU64,
    pub block_parse_tx_new_time_ns: AtomicU64,
    pub block_parse_tx_new_count: AtomicU64,

    // Token cache operations
    pub token_cache_load_time_ns: AtomicU64,
    pub token_cache_load_count: AtomicU64,
    pub token_cache_process_time_ns: AtomicU64,
    pub token_cache_process_count: AtomicU64,

    // Database write operations
    pub db_write_time_ns: AtomicU64,
    pub db_write_count: AtomicU64,

    // Prevout processing
    pub prevout_process_time_ns: AtomicU64,
    pub prevout_process_count: AtomicU64,
    pub prevout_build_time_ns: AtomicU64,
    pub prevout_build_count: AtomicU64,
    pub prevout_inputs_time_ns: AtomicU64,
    pub prevout_inputs_count: AtomicU64,
    pub prevout_cache_time_ns: AtomicU64,
    pub prevout_cache_count: AtomicU64,
    pub prevout_db_fetch_time_ns: AtomicU64,
    pub prevout_db_fetch_count: AtomicU64,
    pub prevout_cache_insert_time_ns: AtomicU64,
    pub prevout_cache_insert_count: AtomicU64,

    // Total block handling time
    pub block_handle_time_ns: AtomicU64,
    pub block_handle_count: AtomicU64,

    // Pre-FIB flushing
    pub prefib_flush_time_ns: AtomicU64,
    pub prefib_flush_count: AtomicU64,

    // Event emission/history dispatch
    pub event_emit_time_ns: AtomicU64,
    pub event_emit_count: AtomicU64,
    pub history_send_time_ns: AtomicU64,
    pub history_send_count: AtomicU64,

    // Idle time between blocks
    pub idle_time_ns: AtomicU64,
    pub idle_count: AtomicU64,

    // Last print time
    last_print: std::sync::Mutex<Instant>,
    last_task_end: std::sync::Mutex<Option<Instant>>,
}

impl IndexingMetrics {
    pub const fn new() -> Self {
        Self {
            block_read_time_ns: AtomicU64::new(0),
            block_read_count: AtomicU64::new(0),
            block_read_xor_time_ns: AtomicU64::new(0),
            block_read_xor_count: AtomicU64::new(0),
            block_read_header_time_ns: AtomicU64::new(0),
            block_read_header_count: AtomicU64::new(0),
            block_read_tx_time_ns: AtomicU64::new(0),
            block_read_tx_count: AtomicU64::new(0),
            block_read_script_time_ns: AtomicU64::new(0),
            block_read_script_count: AtomicU64::new(0),
            block_read_hash_time_ns: AtomicU64::new(0),
            block_read_hash_count: AtomicU64::new(0),
            block_read_varint_time_ns: AtomicU64::new(0),
            block_read_varint_count: AtomicU64::new(0),
            block_read_auxpow_time_ns: AtomicU64::new(0),
            block_read_auxpow_count: AtomicU64::new(0),
            block_parse_time_ns: AtomicU64::new(0),
            block_parse_count: AtomicU64::new(0),
            block_parse_partials_time_ns: AtomicU64::new(0),
            block_parse_partials_count: AtomicU64::new(0),
            block_parse_offsets_time_ns: AtomicU64::new(0),
            block_parse_offsets_count: AtomicU64::new(0),
            block_parse_tx_time_ns: AtomicU64::new(0),
            block_parse_tx_count: AtomicU64::new(0),
            block_parse_tx_offsets_time_ns: AtomicU64::new(0),
            block_parse_tx_offsets_count: AtomicU64::new(0),
            block_parse_tx_inputs_time_ns: AtomicU64::new(0),
            block_parse_tx_inputs_count: AtomicU64::new(0),
            block_parse_tx_new_time_ns: AtomicU64::new(0),
            block_parse_tx_new_count: AtomicU64::new(0),
            token_cache_load_time_ns: AtomicU64::new(0),
            token_cache_load_count: AtomicU64::new(0),
            token_cache_process_time_ns: AtomicU64::new(0),
            token_cache_process_count: AtomicU64::new(0),
            db_write_time_ns: AtomicU64::new(0),
            db_write_count: AtomicU64::new(0),
            prevout_process_time_ns: AtomicU64::new(0),
            prevout_process_count: AtomicU64::new(0),
            prevout_build_time_ns: AtomicU64::new(0),
            prevout_build_count: AtomicU64::new(0),
            prevout_inputs_time_ns: AtomicU64::new(0),
            prevout_inputs_count: AtomicU64::new(0),
            prevout_cache_time_ns: AtomicU64::new(0),
            prevout_cache_count: AtomicU64::new(0),
            prevout_db_fetch_time_ns: AtomicU64::new(0),
            prevout_db_fetch_count: AtomicU64::new(0),
            prevout_cache_insert_time_ns: AtomicU64::new(0),
            prevout_cache_insert_count: AtomicU64::new(0),
            block_handle_time_ns: AtomicU64::new(0),
            block_handle_count: AtomicU64::new(0),
            prefib_flush_time_ns: AtomicU64::new(0),
            prefib_flush_count: AtomicU64::new(0),
            event_emit_time_ns: AtomicU64::new(0),
            event_emit_count: AtomicU64::new(0),
            history_send_time_ns: AtomicU64::new(0),
            history_send_count: AtomicU64::new(0),
            idle_time_ns: AtomicU64::new(0),
            idle_count: AtomicU64::new(0),
            last_print: std::sync::Mutex::new(unsafe { std::mem::zeroed() }),
            last_task_end: std::sync::Mutex::new(None),
        }
    }

    pub fn init(&self) {
        let now = Instant::now();
        *self.last_print.lock().unwrap() = now;
        *self.last_task_end.lock().unwrap() = Some(now);
    }

    pub fn record_block_parse(&self, duration: Duration) {
        self.block_parse_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_partials(&self, duration: Duration) {
        self.block_parse_partials_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_partials_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_offsets(&self, duration: Duration) {
        self.block_parse_offsets_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_offsets_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_tx_loop(&self, duration: Duration) {
        self.block_parse_tx_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_tx_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_tx_offsets(&self, duration: Duration) {
        self.block_parse_tx_offsets_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_tx_offsets_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_tx_inputs(&self, duration: Duration) {
        self.block_parse_tx_inputs_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_tx_inputs_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_parse_tx_new(&self, duration: Duration) {
        self.block_parse_tx_new_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_parse_tx_new_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_token_cache_load(&self, duration: Duration) {
        self.token_cache_load_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.token_cache_load_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_token_cache_process(&self, duration: Duration) {
        self.token_cache_process_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.token_cache_process_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_db_write(&self, duration: Duration) {
        self.db_write_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.db_write_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_process(&self, duration: Duration) {
        self.prevout_process_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_process_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_build(&self, duration: Duration) {
        self.prevout_build_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_build_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_inputs(&self, duration: Duration) {
        self.prevout_inputs_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_inputs_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_cache_lookup(&self, duration: Duration) {
        self.prevout_cache_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_cache_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_db_fetch(&self, duration: Duration) {
        self.prevout_db_fetch_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_db_fetch_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prevout_cache_insert(&self, duration: Duration) {
        self.prevout_cache_insert_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prevout_cache_insert_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_block_handle(&self, duration: Duration) {
        self.block_handle_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.block_handle_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_prefib_flush(&self, duration: Duration) {
        self.prefib_flush_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.prefib_flush_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_event_emit(&self, duration: Duration) {
        self.event_emit_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.event_emit_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_history_send(&self, duration: Duration) {
        self.history_send_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.history_send_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_idle(&self, duration: Duration) {
        self.idle_time_ns.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.idle_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_task_start(&self, start: Instant) {
        let guard = self.last_task_end.lock().unwrap();
        if let Some(last_end) = *guard {
            if start > last_end {
                self.record_idle(start - last_end);
            }
        }
    }

    pub fn note_task_end(&self, end: Instant) {
        let mut guard = self.last_task_end.lock().unwrap();
        *guard = Some(end);
    }

    /// Collect block read metrics from the parser crate
    fn collect_block_read_metrics(&self) {
        let (
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
        ) = nint_blk::BLOCK_READ_METRICS.take();
        self.block_read_time_ns.fetch_add(time_ns, Ordering::Relaxed);
        self.block_read_count.fetch_add(count, Ordering::Relaxed);
        self.block_read_xor_time_ns.fetch_add(xor_ns, Ordering::Relaxed);
        self.block_read_xor_count.fetch_add(xor_count, Ordering::Relaxed);
        self.block_read_header_time_ns.fetch_add(header_ns, Ordering::Relaxed);
        self.block_read_header_count.fetch_add(header_count, Ordering::Relaxed);
        self.block_read_tx_time_ns.fetch_add(tx_ns, Ordering::Relaxed);
        self.block_read_tx_count.fetch_add(tx_count, Ordering::Relaxed);
        self.block_read_script_time_ns.fetch_add(script_ns, Ordering::Relaxed);
        self.block_read_script_count.fetch_add(script_count, Ordering::Relaxed);
        self.block_read_hash_time_ns.fetch_add(hash_ns, Ordering::Relaxed);
        self.block_read_hash_count.fetch_add(hash_count, Ordering::Relaxed);
        self.block_read_varint_time_ns.fetch_add(varint_ns, Ordering::Relaxed);
        self.block_read_varint_count.fetch_add(varint_count, Ordering::Relaxed);
        self.block_read_auxpow_time_ns.fetch_add(auxpow_ns, Ordering::Relaxed);
        self.block_read_auxpow_count.fetch_add(auxpow_count, Ordering::Relaxed);
    }

    /// Print metrics every N seconds, returns true if printed
    pub fn maybe_print(&self, interval_secs: u64) -> bool {
        let mut last_print = self.last_print.lock().unwrap();
        if last_print.elapsed().as_secs() < interval_secs {
            return false;
        }
        *last_print = Instant::now();
        drop(last_print);

        self.print_and_reset();
        true
    }

    pub fn print_and_reset(&self) {
        // Collect metrics from parser crate first
        self.collect_block_read_metrics();

        let block_read_ns = self.block_read_time_ns.swap(0, Ordering::Relaxed);
        let block_read_count = self.block_read_count.swap(0, Ordering::Relaxed);
        let block_read_xor_ns = self.block_read_xor_time_ns.swap(0, Ordering::Relaxed);
        let block_read_xor_count = self.block_read_xor_count.swap(0, Ordering::Relaxed);
        let block_read_header_ns = self.block_read_header_time_ns.swap(0, Ordering::Relaxed);
        let block_read_header_count = self.block_read_header_count.swap(0, Ordering::Relaxed);
        let block_read_tx_ns = self.block_read_tx_time_ns.swap(0, Ordering::Relaxed);
        let block_read_tx_count = self.block_read_tx_count.swap(0, Ordering::Relaxed);
        let block_read_script_ns = self.block_read_script_time_ns.swap(0, Ordering::Relaxed);
        let block_read_script_count = self.block_read_script_count.swap(0, Ordering::Relaxed);
        let block_read_hash_ns = self.block_read_hash_time_ns.swap(0, Ordering::Relaxed);
        let block_read_hash_count = self.block_read_hash_count.swap(0, Ordering::Relaxed);
        let block_read_varint_ns = self.block_read_varint_time_ns.swap(0, Ordering::Relaxed);
        let block_read_varint_count = self.block_read_varint_count.swap(0, Ordering::Relaxed);
        let block_read_auxpow_ns = self.block_read_auxpow_time_ns.swap(0, Ordering::Relaxed);
        let block_read_auxpow_count = self.block_read_auxpow_count.swap(0, Ordering::Relaxed);
        let block_read_varint_ns = self.block_read_varint_time_ns.swap(0, Ordering::Relaxed);
        let block_read_varint_count = self.block_read_varint_count.swap(0, Ordering::Relaxed);
        let block_read_auxpow_ns = self.block_read_auxpow_time_ns.swap(0, Ordering::Relaxed);
        let block_read_auxpow_count = self.block_read_auxpow_count.swap(0, Ordering::Relaxed);
        let block_parse_ns = self.block_parse_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_count = self.block_parse_count.swap(0, Ordering::Relaxed);
        let block_parse_partials_ns = self.block_parse_partials_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_partials_count = self.block_parse_partials_count.swap(0, Ordering::Relaxed);
        let block_parse_offsets_ns = self.block_parse_offsets_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_offsets_count = self.block_parse_offsets_count.swap(0, Ordering::Relaxed);
        let block_parse_tx_ns = self.block_parse_tx_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_tx_count = self.block_parse_tx_count.swap(0, Ordering::Relaxed);
        let block_parse_tx_offsets_ns = self.block_parse_tx_offsets_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_tx_offsets_count = self.block_parse_tx_offsets_count.swap(0, Ordering::Relaxed);
        let block_parse_tx_inputs_ns = self.block_parse_tx_inputs_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_tx_inputs_count = self.block_parse_tx_inputs_count.swap(0, Ordering::Relaxed);
        let block_parse_tx_new_ns = self.block_parse_tx_new_time_ns.swap(0, Ordering::Relaxed);
        let block_parse_tx_new_count = self.block_parse_tx_new_count.swap(0, Ordering::Relaxed);
        let token_load_ns = self.token_cache_load_time_ns.swap(0, Ordering::Relaxed);
        let token_load_count = self.token_cache_load_count.swap(0, Ordering::Relaxed);
        let token_process_ns = self.token_cache_process_time_ns.swap(0, Ordering::Relaxed);
        let token_process_count = self.token_cache_process_count.swap(0, Ordering::Relaxed);
        let db_write_ns = self.db_write_time_ns.swap(0, Ordering::Relaxed);
        let db_write_count = self.db_write_count.swap(0, Ordering::Relaxed);
        let prevout_ns = self.prevout_process_time_ns.swap(0, Ordering::Relaxed);
        let prevout_count = self.prevout_process_count.swap(0, Ordering::Relaxed);
        let prevout_build_ns = self.prevout_build_time_ns.swap(0, Ordering::Relaxed);
        let prevout_build_count = self.prevout_build_count.swap(0, Ordering::Relaxed);
        let prevout_inputs_ns = self.prevout_inputs_time_ns.swap(0, Ordering::Relaxed);
        let prevout_inputs_count = self.prevout_inputs_count.swap(0, Ordering::Relaxed);
        let prevout_cache_ns = self.prevout_cache_time_ns.swap(0, Ordering::Relaxed);
        let prevout_cache_count = self.prevout_cache_count.swap(0, Ordering::Relaxed);
        let prevout_db_fetch_ns = self.prevout_db_fetch_time_ns.swap(0, Ordering::Relaxed);
        let prevout_db_fetch_count = self.prevout_db_fetch_count.swap(0, Ordering::Relaxed);
        let prevout_cache_insert_ns = self.prevout_cache_insert_time_ns.swap(0, Ordering::Relaxed);
        let prevout_cache_insert_count = self.prevout_cache_insert_count.swap(0, Ordering::Relaxed);
        let block_handle_ns = self.block_handle_time_ns.swap(0, Ordering::Relaxed);
        let block_handle_count = self.block_handle_count.swap(0, Ordering::Relaxed);
        let prefib_flush_ns = self.prefib_flush_time_ns.swap(0, Ordering::Relaxed);
        let prefib_flush_count = self.prefib_flush_count.swap(0, Ordering::Relaxed);
        let event_emit_ns = self.event_emit_time_ns.swap(0, Ordering::Relaxed);
        let event_emit_count = self.event_emit_count.swap(0, Ordering::Relaxed);
        let history_send_ns = self.history_send_time_ns.swap(0, Ordering::Relaxed);
        let history_send_count = self.history_send_count.swap(0, Ordering::Relaxed);
        let idle_ns = self.idle_time_ns.swap(0, Ordering::Relaxed);
        let idle_count = self.idle_count.swap(0, Ordering::Relaxed);

        if block_handle_count == 0 {
            return;
        }

        let format_avg = |total_ns: u64, count: u64| -> String {
            if count == 0 {
                "N/A".to_string()
            } else {
                let avg_ms = (total_ns as f64 / count as f64) / 1_000_000.0;
                format!("{:.3}ms", avg_ms)
            }
        };

        let format_total = |total_ns: u64| -> String {
            let total_ms = total_ns as f64 / 1_000_000.0;
            if total_ms > 1000.0 {
                format!("{:.2}s", total_ms / 1000.0)
            } else {
                format!("{:.1}ms", total_ms)
            }
        };

        let total_accounted = block_read_ns + block_parse_ns + token_load_ns + token_process_ns + db_write_ns + prevout_ns + prefib_flush_ns + event_emit_ns + history_send_ns;

        let format_count = |count: u64| -> String {
            if count == 0 {
                "-".to_string()
            } else {
                format!("{}", count)
            }
        };
        let row = |label: &str, count: u64, total_ns: u64, avg: String| -> String {
            format!("║ {:<28} │ {:>10} │ {:>11} │ {:>13} ║", label, format_count(count), format_total(total_ns), avg)
        };

        let table = format!(
            "\n╔══════════════════════════════════════════════════════════════════════╗\n\
             ║                     INDEXING PERFORMANCE METRICS                     ║\n\
             ╠══════════════════════════════════════════════════════════════════════╣\n\
             ║ Blocks processed: {block_handle_count:<10} │ Total handle time: {block_handle_time:<18} ║\n\
             ╠══════════════════════════════════════════════════════════════════════╣\n\
             ║ Operation                   │ Count      │ Total Time  │ Avg/Block     ║\n\
             ╟──────────────────────────────┼────────────┼─────────────┼──────────────╢\n\
             {read_row}\n\
             {parse_row}\n\
             {read_xor_row}\n\
             {read_header_row}\n\
             {read_tx_row}\n\
             {read_script_row}\n\
             {read_hash_row}\n\
             {read_varint_row}\n\
             {read_auxpow_row}\n\
             {parse_partials_row}\n\
             {parse_offsets_row}\n\
             {parse_tx_row}\n\
            {tx_offsets_row}\n\
            {tx_inputs_row}\n\
            {tx_new_row}\n\
            {prevout_row}\n\
            {prevout_details}\
            {token_load_row}\n\
            {token_proc_row}\n\
            {db_write_row}\n\
             {prefib_flush_row}\n\
             {event_emit_row}\n\
             {history_send_row}\n\
             {idle_row}\n\
             ╠══════════════════════════════════════════════════════════════════════╣\n\
             ║ Accounted time: {accounted:<11} │ Overhead: {overhead:<27} ║\n\
             ╚══════════════════════════════════════════════════════════════════════╝",
            block_handle_count = block_handle_count,
            block_handle_time = format_total(block_handle_ns),
            read_row = row("Block Read (blk files)", block_read_count, block_read_ns, format_avg(block_read_ns, block_read_count)),
            parse_row = row("Block Parse", block_parse_count, block_parse_ns, format_avg(block_parse_ns, block_parse_count)),
            prevout_details = if prevout_count > 0 {
                format!(
                    "{}\n{}\n{}\n{}\n{}\n",
                    row("  Prevout: Build Outputs", prevout_build_count, prevout_build_ns, format_avg(prevout_build_ns, prevout_build_count)),
                    row("  Prevout: Collect Inputs", prevout_inputs_count, prevout_inputs_ns, format_avg(prevout_inputs_ns, prevout_inputs_count)),
                    row("  Prevout: Cache Lookup", prevout_cache_count, prevout_cache_ns, format_avg(prevout_cache_ns, prevout_cache_count)),
                    row("  Prevout: DB Fetch", prevout_db_fetch_count, prevout_db_fetch_ns, format_avg(prevout_db_fetch_ns, prevout_db_fetch_count)),
                    row("  Prevout: Cache Insert", prevout_cache_insert_count, prevout_cache_insert_ns, format_avg(prevout_cache_insert_ns, prevout_cache_insert_count))
                )
            } else {
                String::new()
            },
            read_xor_row = row(
                "  Block Read: XOR",
                block_read_xor_count,
                block_read_xor_ns,
                format_avg(block_read_xor_ns, block_read_xor_count)
            ),
            read_header_row = row(
                "  Block Read: Header Parse",
                block_read_header_count,
                block_read_header_ns,
                format_avg(block_read_header_ns, block_read_header_count)
            ),
            read_tx_row = row(
                "  Block Read: Tx Decode",
                block_read_tx_count,
                block_read_tx_ns,
                format_avg(block_read_tx_ns, block_read_tx_count)
            ),
            read_script_row = row(
                "  Block Read: Script Eval",
                block_read_script_count,
                block_read_script_ns,
                format_avg(block_read_script_ns, block_read_script_count)
            ),
            read_hash_row = row(
                "  Block Read: Tx Hash",
                block_read_hash_count,
                block_read_hash_ns,
                format_avg(block_read_hash_ns, block_read_hash_count)
            ),
            read_varint_row = row(
                "  Block Read: Varints",
                block_read_varint_count,
                block_read_varint_ns,
                format_avg(block_read_varint_ns, block_read_varint_count)
            ),
            read_auxpow_row = row(
                "  Block Read: AuxPow Decode",
                block_read_auxpow_count,
                block_read_auxpow_ns,
                format_avg(block_read_auxpow_ns, block_read_auxpow_count)
            ),
            parse_partials_row = row(
                "  Parse: Load Partials",
                block_parse_partials_count,
                block_parse_partials_ns,
                format_avg(block_parse_partials_ns, block_parse_partials_count)
            ),
            parse_offsets_row = row(
                "  Parse: Load Offsets",
                block_parse_offsets_count,
                block_parse_offsets_ns,
                format_avg(block_parse_offsets_ns, block_parse_offsets_count)
            ),
            parse_tx_row = row(
                "  Parse: Tx Loop",
                block_parse_tx_count,
                block_parse_tx_ns,
                format_avg(block_parse_tx_ns, block_parse_tx_count)
            ),
            tx_offsets_row = row(
                "    Tx Loop: Offsets Prep",
                block_parse_tx_offsets_count,
                block_parse_tx_offsets_ns,
                format_avg(block_parse_tx_offsets_ns, block_parse_tx_offsets_count)
            ),
            tx_inputs_row = row(
                "    Tx Loop: Input Handling",
                block_parse_tx_inputs_count,
                block_parse_tx_inputs_ns,
                format_avg(block_parse_tx_inputs_ns, block_parse_tx_inputs_count)
            ),
            tx_new_row = row(
                "    Tx Loop: New Inscriptions",
                block_parse_tx_new_count,
                block_parse_tx_new_ns,
                format_avg(block_parse_tx_new_ns, block_parse_tx_new_count)
            ),
            prevout_row = row("Prevout Processing", prevout_count, prevout_ns, format_avg(prevout_ns, prevout_count)),
            token_load_row = row("Token Cache Load", token_load_count, token_load_ns, format_avg(token_load_ns, token_load_count)),
            token_proc_row = row(
                "Token Cache Process",
                token_process_count,
                token_process_ns,
                format_avg(token_process_ns, token_process_count)
            ),
            db_write_row = row("Database Write", db_write_count, db_write_ns, format_avg(db_write_ns, db_write_count)),
            prefib_flush_row = row("Pre-FIB Flush", prefib_flush_count, prefib_flush_ns, format_avg(prefib_flush_ns, prefib_flush_count)),
            event_emit_row = row("Event Emit / Token Apply", event_emit_count, event_emit_ns, format_avg(event_emit_ns, event_emit_count)),
            history_send_row = row("History Dispatch", history_send_count, history_send_ns, format_avg(history_send_ns, history_send_count)),
            idle_row = row("Idle Wait", idle_count, idle_ns, format_avg(idle_ns, idle_count)),
            accounted = format_total(total_accounted),
            overhead = format_total(block_handle_ns.saturating_sub(total_accounted)),
        );

        tracing::info!(target: "timing", "{}", table);
    }
}

/// Global metrics instance
pub static INDEXING_METRICS: IndexingMetrics = IndexingMetrics::new();

/// Helper macro for timing a block of code
#[macro_export]
macro_rules! time_operation {
    ($metrics_fn:ident, $code:expr) => {{
        let start = std::time::Instant::now();
        let result = $code;
        $crate::utils::timing::INDEXING_METRICS.$metrics_fn(start.elapsed());
        result
    }};
}
