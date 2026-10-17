//! Volume capacity for apps: the total, free and available bytes of the filesystem that holds a path.
//!
//! ```ignore
//! let c = volume::capacity(Path::new(&document_path))?;
//! if c.available < needed { /* tell the user the disk is full */ }
//! ```
//! The model and the call are in `bifrost_core::volume`; the shell's disk readings use the same call. Reads only:
//! nothing is opened for writing and no quota is set. A path that does not exist is an error, not a zero reading.
pub use bifrost_core::volume::{capacity, Capacity};

use std::io;
use std::path::{Path, PathBuf};

/// True when a new reading differs from the last one by at least `min_change` bytes of free space, or the size of the
/// volume changed at all. A `min_change` of 0 reports every change in free space.
pub fn differs(last: Capacity, next: Capacity, min_change: u64) -> bool {
    last.total != next.total || last.free.abs_diff(next.free) >= min_change.max(1)
}

/// Reports a volume's free space when it changes (the app calls `poll` from its own run loop; there is no callback).
#[derive(Debug)]
pub struct CapacityWatch {
    path: PathBuf,
    last: Option<Capacity>,
}

impl CapacityWatch {
    /// A watch on the volume that holds `path`. The first poll always reports.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        CapacityWatch { path: path.into(), last: None }
    }

    /// The path the watch reads.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the volume and returns the reading when it differs from the last one by at least `min_change` bytes
    /// (see [`differs`]). A read that fails is returned as an error and the last reading is kept.
    pub fn poll(&mut self, min_change: u64) -> io::Result<Option<Capacity>> {
        let next = capacity(&self.path)?;
        let report = match self.last {
            None => true,
            Some(last) => differs(last, next, min_change),
        };
        if report {
            self.last = Some(next);
            Ok(Some(next))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(total: u64, free: u64) -> Capacity {
        Capacity { total, free, available: free }
    }

    #[test]
    fn a_change_below_the_threshold_is_not_reported() {
        assert!(!differs(cap(100, 50), cap(100, 45), 10));
        assert!(differs(cap(100, 50), cap(100, 40), 10));
        assert!(differs(cap(100, 50), cap(100, 55), 5), "growth counts too");
    }

    #[test]
    fn a_zero_threshold_reports_every_change_in_free_space() {
        assert!(differs(cap(100, 50), cap(100, 49), 0));
        assert!(!differs(cap(100, 50), cap(100, 50), 0), "no change is never reported");
    }

    #[test]
    fn a_changed_size_is_always_reported() {
        assert!(differs(cap(100, 50), cap(200, 50), u64::MAX));
    }

    #[test]
    fn the_first_poll_reports_and_an_unchanged_volume_does_not() {
        let mut watch = CapacityWatch::new("/");
        assert!(watch.poll(u64::MAX).expect("the root volume is mounted").is_some());
        assert!(watch.poll(u64::MAX).expect("read again").is_none(), "nothing changed beyond u64::MAX bytes");
    }
}

