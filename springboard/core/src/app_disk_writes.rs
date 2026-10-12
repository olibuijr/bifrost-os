//! Per-app disk writes and storage use as a pure model (docs/parity-matrix, metrickit). It keeps three things per app:
//! the running total of bytes written to long-term storage, the storage an app's files occupy (binary, cache and clone
//! sizes and a file count), and a windowed check that reports when the bytes written inside a window exceed a limit.
//!
//! Callers supply every figure. Nothing here reads a file, a clock or a counter. Byte totals saturate at the maximum,
//! and a window whose counter went backwards (the counter restarted) reads as zero rather than wrapping.

use std::collections::BTreeMap;

/// The storage one app's files occupy, in bytes, with the number of files they make up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Storage {
    pub file_count: u64,
    pub binary_bytes: u64,
    pub cache_bytes: u64,
    pub clone_bytes: u64,
}

impl Storage {
    /// The three size fields added together, saturating at the maximum.
    pub fn size_total(&self) -> u64 {
        self.binary_bytes.saturating_add(self.cache_bytes).saturating_add(self.clone_bytes)
    }
}

/// A windowed disk-write exception: the bytes an app wrote during the window went over the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteException {
    pub bytes_written: u64,
    pub limit_bytes: u64,
}

/// Checks one window. `written_now` and `written_at_start` are the app's running write totals at the two ends of the
/// window. A limit of zero means no limit, so no exception is ever reported. The exception is reported only when the
/// bytes written are strictly above the limit.
pub fn check_window(written_now: u64, written_at_start: u64, limit_bytes: u64) -> Option<WriteException> {
    if limit_bytes == 0 {
        return None;
    }
    let bytes_written = written_now.saturating_sub(written_at_start);
    if bytes_written > limit_bytes {
        Some(WriteException { bytes_written, limit_bytes })
    } else {
        None
    }
}

/// The running disk-write totals and storage figures of every app that has reported to the model.
#[derive(Debug, Default)]
pub struct DiskLedger {
    writes: BTreeMap<String, u64>,
    storage: BTreeMap<String, Storage>,
}

impl DiskLedger {
    pub fn new() -> DiskLedger {
        DiskLedger::default()
    }

    /// Adds `bytes` to the app's running write total. Zero bytes change nothing.
    pub fn record_write(&mut self, app: &str, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let total = self.writes.entry(app.to_string()).or_insert(0);
        *total = total.saturating_add(bytes);
    }

    /// The app's running write total. An app that never wrote has zero.
    pub fn written(&self, app: &str) -> u64 {
        self.writes.get(app).copied().unwrap_or(0)
    }

    /// Replaces the app's storage figures with a new snapshot. The last snapshot wins.
    pub fn set_storage(&mut self, app: &str, storage: Storage) {
        self.storage.insert(app.to_string(), storage);
    }

    /// The app's last storage snapshot, if it has one.
    pub fn storage(&self, app: &str) -> Option<&Storage> {
        self.storage.get(app)
    }

    /// The apps with a running write total, in sorted order.
    pub fn apps_with_writes(&self) -> Vec<&str> {
        self.writes.keys().map(|name| name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_accumulate_per_app() {
        let mut ledger = DiskLedger::new();
        ledger.record_write("editor", 300);
        ledger.record_write("editor", 200);
        ledger.record_write("photos", 5);
        assert_eq!(ledger.written("editor"), 500);
        assert_eq!(ledger.written("photos"), 5);
        assert_eq!(ledger.written("unknown"), 0);
        assert_eq!(ledger.apps_with_writes(), vec!["editor", "photos"]);
    }

    #[test]
    fn zero_bytes_create_no_entry() {
        let mut ledger = DiskLedger::new();
        ledger.record_write("quiet", 0);
        assert!(ledger.apps_with_writes().is_empty());
    }

    #[test]
    fn writes_saturate_at_the_maximum() {
        let mut ledger = DiskLedger::new();
        ledger.record_write("bulk", u64::MAX);
        ledger.record_write("bulk", 1);
        assert_eq!(ledger.written("bulk"), u64::MAX);
    }

    #[test]
    fn a_window_above_the_limit_is_an_exception() {
        // 3000 - 1000 = 2000 bytes written in the window, limit 1500.
        let e = check_window(3_000, 1_000, 1_500).unwrap();
        assert_eq!(e.bytes_written, 2_000);
        assert_eq!(e.limit_bytes, 1_500);
    }

    #[test]
    fn a_window_exactly_at_the_limit_is_not_an_exception() {
        assert_eq!(check_window(2_500, 1_000, 1_500), None);
    }

    #[test]
    fn a_zero_limit_means_no_limit() {
        assert_eq!(check_window(u64::MAX, 0, 0), None);
    }

    #[test]
    fn a_counter_that_went_backwards_reads_as_no_writes() {
        // The counter restarted: 100 now against 5000 at the start of the window.
        assert_eq!(check_window(100, 5_000, 10), None);
    }

    #[test]
    fn storage_total_adds_the_three_sizes() {
        let s = Storage { file_count: 9, binary_bytes: 10, cache_bytes: 20, clone_bytes: 30 };
        assert_eq!(s.size_total(), 60);
        assert_eq!(Storage::default().size_total(), 0);
    }

    #[test]
    fn the_last_storage_snapshot_wins() {
        let mut ledger = DiskLedger::new();
        ledger.set_storage("notes", Storage { file_count: 1, binary_bytes: 1, cache_bytes: 0, clone_bytes: 0 });
        ledger.set_storage("notes", Storage { file_count: 4, binary_bytes: 2, cache_bytes: 3, clone_bytes: 0 });
        let s = ledger.storage("notes").unwrap();
        assert_eq!(s.file_count, 4);
        assert_eq!(s.size_total(), 5);
        assert!(ledger.storage("missing").is_none());
    }
}
