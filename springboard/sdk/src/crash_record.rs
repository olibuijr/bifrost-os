//! Crash records the shell writes into its data directory: the `crash.log` lines (panics, watchdog exits and quits of
//! apps that fail) and the `last_exit` marker of a controlled exit. This is the reading side of the format that
//! `springboard/src/bare/crashlog.rs` writes.
//!
//! The parsers are pure. They take the text of a file and return typed records; a malformed line is an [`Error`] that
//! carries the line, so a caller can skip it or show it. Reading the files is the caller's job, so the same code reads a
//! log from a diagnostics bundle or from a copy made on another machine. The lenient listing that the shell and
//! [`crate::exception_handler::crash_records`] use is `springboard::shell::diag::parse_line`, which drops lines it cannot
//! read; this module returns each unreadable line as an [`Error`] instead.

use std::fmt;

/// The crash log in the data directory.
pub const CRASH_LOG_FILE: &str = "crash.log";
/// The rotated crash log: the previous file, kept once the current one reached its size limit.
pub const ROTATED_CRASH_LOG_FILE: &str = "crash.log.old";
/// The marker a controlled exit leaves: `<status> <reason>`.
pub const LAST_EXIT_FILE: &str = "last_exit";
/// The marker touched at every start.
pub const LAST_START_FILE: &str = "last_start";

/// Why a record could not be read. Each variant carries the offending text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The line does not begin with a `[time clock]` stamp.
    MissingStamp(String),
    /// The time in the stamp is not a whole number of seconds.
    BadTime(String),
    /// The line has no `thread '<name>': ` part after the stamp.
    MissingThread(String),
    /// The `last_exit` text is not a status number followed by a reason.
    BadExit(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingStamp(line) => write!(f, "crash entry has no [time clock] stamp: {line}"),
            Error::BadTime(line) => write!(f, "crash entry time is not a whole number of seconds: {line}"),
            Error::MissingThread(line) => write!(f, "crash entry has no thread name: {line}"),
            Error::BadExit(text) => write!(f, "last_exit is not '<status> <reason>': {text}"),
        }
    }
}

impl std::error::Error for Error {}

/// One crash.log entry. The message keeps any line breaks as ` | `, the way the shell wrote them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashEntry {
    /// Seconds since the Unix epoch when the entry was written.
    pub unix_time: i64,
    /// The name of the thread that wrote the entry, or `unnamed`.
    pub thread: String,
    pub message: String,
}

/// Reads one crash.log line: `[<unix time> <clock>] thread '<name>': <message>`.
pub fn parse_entry(line: &str) -> Result<CrashEntry, Error> {
    let rest = line.strip_prefix('[').ok_or_else(|| Error::MissingStamp(line.into()))?;
    let (stamp, rest) = rest.split_once(']').ok_or_else(|| Error::MissingStamp(line.into()))?;
    let unix_time = stamp
        .split_whitespace()
        .next()
        .and_then(|t| t.parse::<i64>().ok())
        .ok_or_else(|| Error::BadTime(line.into()))?;
    let rest = rest.trim_start().strip_prefix("thread '").ok_or_else(|| Error::MissingThread(line.into()))?;
    let (thread, message) = rest.split_once("': ").ok_or_else(|| Error::MissingThread(line.into()))?;
    Ok(CrashEntry { unix_time, thread: thread.to_string(), message: message.to_string() })
}

/// Reads a whole crash.log. Blank lines are skipped; every other line gives one result, in file order, so a partly
/// written last line shows up as an error instead of hiding the entries before it.
pub fn parse_log(text: &str) -> Vec<Result<CrashEntry, Error>> {
    text.lines().filter(|line| !line.trim().is_empty()).map(parse_entry).collect()
}

/// Where a panic happened, read from the message of an entry that starts with `panic: `.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Panic {
    /// The panic message and the source location it was raised at.
    At { message: String, file: String, line: u32, column: u32 },
    /// The panic message; the location was not known when it was written.
    UnknownLocation { message: String },
}

/// Reads a panic message: `panic: <message> at <file>:<line>:<column>` or `panic: <message> at unknown location`.
/// Returns `None` for any other message, including a location that does not end in line and column numbers.
pub fn parse_panic(message: &str) -> Option<Panic> {
    let body = message.strip_prefix("panic: ")?;
    if let Some(text) = body.strip_suffix(" at unknown location") {
        return Some(Panic::UnknownLocation { message: text.to_string() });
    }
    let (text, location) = body.rsplit_once(" at ")?;
    let mut parts = location.rsplitn(3, ':');
    let column = parts.next()?.parse::<u32>().ok()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let file = parts.next().filter(|f| !f.is_empty())?;
    Some(Panic::At { message: text.to_string(), file: file.to_string(), line, column })
}

/// The `last_exit` marker: how a controlled exit ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastExit {
    pub status: i32,
    pub reason: String,
}

/// Reads the `last_exit` marker. The status is the first word; the reason is the rest of the line.
pub fn parse_last_exit(text: &str) -> Result<LastExit, Error> {
    let text = text.trim_end();
    let (status, reason) = text.split_once(' ').ok_or_else(|| Error::BadExit(text.into()))?;
    let status = status.parse::<i32>().map_err(|_| Error::BadExit(text.into()))?;
    Ok(LastExit { status, reason: reason.to_string() })
}

/// True when a start found the `last_start` marker of an earlier run but no `last_exit`: that run ended without a
/// trace (a panic abort, a kill or a power loss). A start with no earlier marker is a first start, not a crash.
pub fn ended_uncleanly(had_start: bool, last_exit: Option<&LastExit>) -> bool {
    had_start && last_exit.is_none()
}
