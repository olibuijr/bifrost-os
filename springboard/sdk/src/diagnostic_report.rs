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
