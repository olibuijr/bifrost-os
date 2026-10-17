//! Diagnostic reports for apps: the crash and stall records of the shell's crash log, as typed values.
//!
//! The shell appends one line per panic and one line per UI loop stall, and it records the exit reason of a process
//! that stalled. [`DiagnosticReport::from_crash_log`] reads log text with the shell's own line parser, so the line
//! format has one definition. [`longest_stall`] gives the hang duration the log can show, in whole seconds. The
//! module does not open the shell's log file, and the shell does not yet call an app when a record is written.

use springboard::shell::diag::parse_log;
use std::path::Path;

/// The text the shell writes for a UI loop that stopped beating.
const STALL_PREFIX: &str = "ui loop stalled for ";

/// What a record describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// The UI loop did not beat for the stall limit. `seconds` is the whole number the shell recorded.
    Stall { seconds: u64 },
    /// Any other record: a panic, whose signature holds the location and message, or an exit reason.
    Crash,
}

/// One record of the crash log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticReport {
    /// Seconds since the Unix epoch when the shell wrote the record.
    pub unix: i64,
    pub kind: DiagnosticKind,
    /// A short description of the record, at most 60 characters.
    pub signature: String,
}

impl DiagnosticReport {
    /// Every record in `text`, oldest first. Lines that do not parse are skipped.
    pub fn from_crash_log(text: &str) -> Vec<DiagnosticReport> {
        parse_log(text, Path::new("crash.log"))
            .into_iter()
            .map(|entry| {
                let kind = match stall_seconds(&entry.signature) {
                    Some(seconds) => DiagnosticKind::Stall { seconds },
                    None => DiagnosticKind::Crash,
                };
                DiagnosticReport { unix: entry.unix, kind, signature: entry.signature }
            })
            .collect()
    }
}

/// The seconds in a stall signature such as "ui loop stalled for 21s", or `None` for any other text.
fn stall_seconds(signature: &str) -> Option<u64> {
    signature.strip_prefix(STALL_PREFIX)?.strip_suffix('s')?.parse().ok()
}

/// The longest UI stall in `reports`, in whole seconds, or `None` when the list holds no stall.
pub fn longest_stall(reports: &[DiagnosticReport]) -> Option<u64> {
    reports
        .iter()
        .filter_map(|report| match report.kind {
            DiagnosticKind::Stall { seconds } => Some(seconds),
            DiagnosticKind::Crash => None,
        })
        .max()
}

/// Upper edges of the stall buckets, in whole seconds. Bucket `i` holds stalls of `EDGES[i - 1]` up to but not
/// including `EDGES[i]`; the first starts at 0 and the last has no upper edge.
const STALL_EDGES: [u64; 4] = [5, 10, 30, 60];

/// One bucket of the stall histogram. `upper` is `None` for the open-ended last bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallBucket {
    pub lower: u64,
    pub upper: Option<u64>,
    pub count: usize,
}

/// Counts the stalls in `reports` per bucket: 0 to 5 s, 5 to 10 s, 10 to 30 s, 30 to 60 s and 60 s and longer.
/// Crash records are not counted. The buckets always come back in order, so an empty bucket shows as 0.
pub fn stall_histogram(reports: &[DiagnosticReport]) -> Vec<StallBucket> {
    let mut buckets: Vec<StallBucket> = (0..=STALL_EDGES.len())
        .map(|i| StallBucket {
            lower: if i == 0 { 0 } else { STALL_EDGES[i - 1] },
            upper: STALL_EDGES.get(i).copied(),
            count: 0,
        })
        .collect();
    for report in reports {
        if let DiagnosticKind::Stall { seconds } = report.kind {
            let index = STALL_EDGES.iter().position(|&edge| seconds < edge).unwrap_or(STALL_EDGES.len());
            buckets[index].count += 1;
        }
    }
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "[1790000000 14:05:09] thread 'main': panic: index out of bounds at src/shell/mod.rs:812:9\n\
        garbage line\n\
        [1790000005 14:05:14] thread 'watchdog': ui loop stalled for 21s\n\
        [1790000090 14:06:39] thread 'watchdog': ui loop stalled for 34s\n";

    #[test]
    fn a_stall_line_decodes_to_its_whole_seconds() {
        let reports = DiagnosticReport::from_crash_log(LOG);
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[1].kind, DiagnosticKind::Stall { seconds: 21 });
        assert_eq!(reports[1].unix, 1_790_000_005);
        assert_eq!(reports[1].signature, "ui loop stalled for 21s");
    }

    #[test]
    fn a_panic_line_is_a_crash_with_its_location() {
        let reports = DiagnosticReport::from_crash_log(LOG);
        assert_eq!(reports[0].kind, DiagnosticKind::Crash);
        assert_eq!(reports[0].signature, "src/shell/mod.rs:812 index out of bounds");
    }

    #[test]
    fn the_longest_stall_is_the_largest_seconds_value() {
        let reports = DiagnosticReport::from_crash_log(LOG);
        assert_eq!(longest_stall(&reports), Some(34));
    }

    #[test]
    fn a_log_without_a_stall_has_no_longest_stall() {
        let crash_only = "[1790000000 14:05:09] thread 'main': panic: src/a.rs:1:1 boom\n";
        assert_eq!(longest_stall(&DiagnosticReport::from_crash_log(crash_only)), None);
        assert_eq!(longest_stall(&[]), None);
    }

    #[test]
    fn a_stall_signature_with_a_bad_number_is_a_crash() {
        assert_eq!(stall_seconds("ui loop stalled for xs"), None);
        assert_eq!(stall_seconds("ui loop stalled for 9s"), Some(9));
        assert_eq!(stall_seconds("exit reason"), None);
    }

    #[test]
    fn stalls_fall_into_their_buckets() {
        let reports = DiagnosticReport::from_crash_log(
            "[1790000000 14:05:09] thread 'watchdog': ui loop stalled for 4s\n\
             [1790000001 14:05:10] thread 'watchdog': ui loop stalled for 5s\n\
             [1790000002 14:05:11] thread 'watchdog': ui loop stalled for 21s\n\
             [1790000003 14:05:12] thread 'watchdog': ui loop stalled for 34s\n\
             [1790000004 14:05:13] thread 'watchdog': ui loop stalled for 90s\n\
             [1790000005 14:05:14] thread 'main': panic: src/a.rs:1:1 boom\n",
        );
        let buckets = stall_histogram(&reports);
        let counts: Vec<usize> = buckets.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![1, 1, 1, 1, 1]);
        assert_eq!(buckets[0], StallBucket { lower: 0, upper: Some(5), count: 1 });
        assert_eq!(buckets[4], StallBucket { lower: 60, upper: None, count: 1 });
    }

    #[test]
    fn an_empty_log_gives_empty_buckets_in_order() {
        let buckets = stall_histogram(&[]);
        assert_eq!(buckets.len(), 5);
        assert!(buckets.iter().all(|b| b.count == 0));
        assert_eq!(buckets[2].lower, 10);
        assert_eq!(buckets[2].upper, Some(30));
    }
}
