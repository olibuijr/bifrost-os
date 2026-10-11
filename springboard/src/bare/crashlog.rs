//! Crash capture and exit bookkeeping in the data directory.
//!
//! - `DIR/crash.log`: panics (the release profile aborts, so the hook writes
//!   before the abort) and watchdog exits; rotated at 256 KB to `crash.log.old`.
//! - `DIR/last_exit`: `<status> <reason>\n`, written on every controlled exit.
//! - `DIR/last_start`: marker touched at every start. A start that finds the
//!   marker but no `last_exit` knows the previous run died uncleanly (panic
//!   abort, kill, power loss), see `previous_run_note`.

use super::sys;
use std::fs;
use std::io::Write;

/// crash.log is rotated once it reaches this size.
pub const MAX_LOG_BYTES: u64 = 256 * 1024;

/// Whether a log of `len` bytes must be rotated before appending.
pub fn should_rotate(len: u64) -> bool {
    len >= MAX_LOG_BYTES
}

/// One crash.log entry; a single line so the file stays greppable.
pub fn format_entry(stamp: &str, thread: &str, msg: &str) -> String {
    format!("[{stamp}] thread '{thread}': {}\n", msg.replace('\n', " | "))
}

/// The panic line without a backtrace (none is available under panic=abort).
pub fn format_panic(msg: &str, location: Option<(&str, u32, u32)>) -> String {
    match location {
        Some((file, line, col)) => format!("panic: {msg} at {file}:{line}:{col}"),
        None => format!("panic: {msg} at unknown location"),
    }
}

/// What a start learns about the previous run; None when it ended cleanly or
/// there was no earlier start.
pub fn note_for(had_start: bool, last_exit: Option<&str>) -> Option<String> {
    match (had_start, last_exit) {
        (true, None) => Some("previous run ended without last_exit: unclean death (panic, kill or power loss)".into()),
        _ => None,
    }
}

fn stamp() -> String {
    let (unix, _) = sys::wall_time();
    let (h, m, s, ..) = sys::local_time();
    format!("{unix} {h:02}:{m:02}:{s:02}")
}

/// Appends one entry to DIR/crash.log, rotating first when it is full.
pub fn append(dir: &str, msg: &str) {
    let path = format!("{dir}/crash.log");
    if fs::metadata(&path).is_ok_and(|m| should_rotate(m.len())) {
        let _ = fs::rename(&path, format!("{path}.old"));
    }
    let thread = std::thread::current();
    let entry = format_entry(&stamp(), thread.name().unwrap_or("unnamed"), msg);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(entry.as_bytes());
        let _ = f.sync_all();
    }
}

/// Installs the panic hook; call it as early as `dir` is known.
pub fn install(dir: &str) {
    let dir = dir.to_string();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "non-string payload".into());
        let loc = info.location().map(|l| (l.file(), l.line(), l.column()));
        append(&dir, &format_panic(&msg, loc));
        previous(info);
    }));
}

/// Records a controlled exit; call it right before `process::exit`.
pub fn write_last_exit(dir: &str, status: i32, reason: &str) {
    let _ = fs::write(format!("{dir}/last_exit"), format!("{status} {reason}\n"));
}

/// Consumes the previous run's last_exit, marks this start, and returns a note
/// to log when the previous run died without a trace.
pub fn previous_run_note(dir: &str) -> Option<String> {
    let start = format!("{dir}/last_start");
    let exit = format!("{dir}/last_exit");
    let had_start = fs::metadata(&start).is_ok();
    let last = fs::read_to_string(&exit).ok();
    let _ = fs::remove_file(&exit);
    let _ = fs::write(&start, format!("{}\n", stamp()));
    note_for(had_start, last.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_256_kb() {
        assert!(!should_rotate(MAX_LOG_BYTES - 1));
        assert!(should_rotate(MAX_LOG_BYTES));
    }

    #[test]
    fn entries_stay_on_one_line() {
        let e = format_entry("1 00:00:01", "main", "a\nb");
        assert_eq!(e, "[1 00:00:01] thread 'main': a | b\n");
    }

    #[test]
    fn panic_line_has_message_and_location() {
        assert_eq!(format_panic("boom", Some(("a.rs", 3, 9))), "panic: boom at a.rs:3:9");
        assert_eq!(format_panic("boom", None), "panic: boom at unknown location");
    }

    #[test]
    fn only_a_start_without_last_exit_is_unclean() {
        assert!(note_for(true, None).is_some());
        assert!(note_for(true, Some("4 restore")).is_none());
        assert!(note_for(false, None).is_none(), "first ever start");
    }
}
