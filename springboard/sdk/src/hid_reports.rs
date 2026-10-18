//! Raw input reports from a HID device, read through its `hidraw` node (`/dev/hidrawN`). One `read` on the node returns
//! one report, with its report ID first when the device's descriptor uses IDs. This module finds the nodes, reads
//! reports, and keeps a bounded queue of them, so a slow reader loses the oldest reports and knows how many it lost.
//! It does not decode a report: the report descriptor belongs to the device's own parser.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The largest report kept. A HID report on USB is at most 64 bytes of input; a larger one is cut.
pub const MAX_REPORT_BYTES: usize = 1024;

/// One report and the node it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawReport {
    /// The node's index: `3` for `/dev/hidraw3`.
    pub device: u32,
    /// The report's bytes, report ID first when the device uses IDs.
    pub bytes: Vec<u8>,
}

/// The node path of `device`, such as `/dev/hidraw3`.
pub fn node_path(device: u32) -> PathBuf {
    PathBuf::from(format!("/dev/hidraw{device}"))
}

/// The index of a `hidrawN` name (`hidraw3` gives 3), or None for anything else.
pub fn index_of(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("hidraw")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The `hidraw` indexes present in `dir` (normally `/dev`), in ascending order.
pub fn devices_in(dir: &Path) -> io::Result<Vec<u32>> {
    let mut found: Vec<u32> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().and_then(|e| index_of(&e.file_name().to_string_lossy())))
        .collect();
    found.sort_unstable();
    Ok(found)
}

/// Reads one report from `source`. An empty read (the device went away) is an error; a report longer than
/// [`MAX_REPORT_BYTES`] is cut to it.
pub fn read_report(device: u32, source: &mut impl Read) -> io::Result<RawReport> {
    let mut buffer = vec![0u8; MAX_REPORT_BYTES];
    let n = source.read(&mut buffer)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the HID device closed"));
    }
    buffer.truncate(n);
    Ok(RawReport { device, bytes: buffer })
}

/// A bounded queue of reports. When it is full the oldest report goes, and [`ReportQueue::dropped`] counts how many
/// were lost since the last [`ReportQueue::take_dropped`].
#[derive(Debug)]
pub struct ReportQueue {
    capacity: usize,
    reports: std::collections::VecDeque<RawReport>,
    dropped: u64,
}

impl ReportQueue {
    /// A queue holding at most `capacity` reports (at least one).
    pub fn new(capacity: usize) -> ReportQueue {
        ReportQueue { capacity: capacity.max(1), reports: std::collections::VecDeque::new(), dropped: 0 }
    }

    /// Adds a report, dropping the oldest one when the queue is full.
    pub fn push(&mut self, report: RawReport) {
        if self.reports.len() == self.capacity {
            self.reports.pop_front();
            self.dropped += 1;
        }
        self.reports.push_back(report);
    }

    /// Takes the oldest report.
    pub fn pop(&mut self) -> Option<RawReport> {
        self.reports.pop_front()
    }

    pub fn len(&self) -> usize {
        self.reports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }

    /// Reports lost to a full queue since the last call; the count then starts again.
    pub fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_node_name_gives_its_index_and_nothing_else_does() {
        assert_eq!(index_of("hidraw3"), Some(3));
        assert_eq!(index_of("hidraw0"), Some(0));
        assert_eq!(index_of("hidraw"), None);
        assert_eq!(index_of("hidraw3x"), None);
        assert_eq!(index_of("input3"), None);
        assert_eq!(node_path(3), PathBuf::from("/dev/hidraw3"));
    }

    #[test]
    fn devices_are_listed_in_order_from_a_directory() {
        let dir = std::env::temp_dir().join(format!("hid-reports-dev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["hidraw10", "hidraw2", "tty0", "hidrawX"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert_eq!(devices_in(&dir).unwrap(), vec![2, 10]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_read_is_one_report_with_its_id_first() {
        let mut source = Cursor::new(vec![0x01, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let report = read_report(3, &mut source).unwrap();
        assert_eq!(report.device, 3);
        assert_eq!(report.bytes, vec![0x01, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn a_closed_device_is_an_error_not_an_empty_report() {
        let mut source = Cursor::new(Vec::<u8>::new());
        assert_eq!(read_report(3, &mut source).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_counts_the_loss() {
        let report = |n: u8| RawReport { device: 1, bytes: vec![n] };
        let mut queue = ReportQueue::new(2);
        queue.push(report(1));
        queue.push(report(2));
        queue.push(report(3));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.take_dropped(), 1);
        assert_eq!(queue.take_dropped(), 0, "the count starts again");
        assert_eq!(queue.pop(), Some(report(2)));
        assert_eq!(queue.pop(), Some(report(3)));
        assert!(queue.is_empty());
    }
}
