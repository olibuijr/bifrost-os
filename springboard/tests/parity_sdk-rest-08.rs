//! Parity sdk-rest-08: headless tests of the shell's crash log bookkeeping in temporary directories. They cover the
//! appends that record a failure, the rotation at the size limit, the unclean-exit note at the next start, and the
//! controlled-exit record. No test installs a panic hook, starts a process or touches a device.

use std::fs;
use std::path::Path;

use springboard::bare::crashlog::{append, format_panic, previous_run_note, try_append, try_write_last_exit, write_last_exit, MAX_LOG_BYTES};

fn log_lines(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("crash.log")).unwrap().lines().map(String::from).collect()
}

// --- capture: the crash log -------------------------------------------------------------------------------------------

#[test]
fn an_append_writes_one_line_to_the_crash_log() {
    let dir = tempfile::tempdir().unwrap();
    try_append(dir.path().to_str().unwrap(), "panic: boom at a.rs:1:2").unwrap();
    let lines = log_lines(dir.path());
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with("': panic: boom at a.rs:1:2"), "line was {:?}", lines[0]);
}

#[test]
fn an_append_into_a_missing_directory_reports_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    assert!(try_append(missing.to_str().unwrap(), "lost").is_err());
    assert!(!missing.exists(), "nothing creates the directory");
}

#[test]
fn the_ignoring_append_does_not_fail_on_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    append(missing.to_str().unwrap(), "lost");
    assert!(!missing.join("crash.log").exists());
}

#[test]
fn a_full_crash_log_is_rotated_before_the_next_append() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("crash.log"), vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
    try_append(dir.path().to_str().unwrap(), "after").unwrap();
    let rotated = fs::metadata(dir.path().join("crash.log.old")).unwrap();
    assert_eq!(rotated.len(), MAX_LOG_BYTES, "the full log moved to crash.log.old");
    let lines = log_lines(dir.path());
    assert_eq!(lines.len(), 1, "the new log holds only the new entry");
    assert!(lines[0].ends_with("': after"));
}

#[test]
fn a_panic_line_names_the_message_and_its_location() {
    assert_eq!(format_panic("boom", Some(("src/a.rs", 3, 9))), "panic: boom at src/a.rs:3:9");
    assert_eq!(format_panic("boom", None), "panic: boom at unknown location");
}

// --- unclean exit: the note at the next start -------------------------------------------------------------------------

#[test]
fn the_first_start_in_a_directory_has_no_previous_run_to_report() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(previous_run_note(dir.path()), None);
    assert!(dir.path().join("last_start").exists(), "the start marker is written");
}

#[test]
fn a_run_that_recorded_a_controlled_exit_is_clean_at_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    assert_eq!(previous_run_note(dir.path()), None);
    try_write_last_exit(path, 0, "quit").unwrap();
    assert_eq!(previous_run_note(dir.path()), None);
}

#[test]
fn a_run_without_a_controlled_exit_is_unclean_at_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(previous_run_note(dir.path()), None);
    let note = previous_run_note(dir.path()).expect("the earlier start left no exit record");
    assert!(note.contains("unclean"), "note was {note:?}");
}

#[test]
fn a_start_reads_and_clears_the_previous_exit_record() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("last_exit"), "4 restore\n").unwrap();
    assert_eq!(previous_run_note(dir.path()), None);
    assert!(!dir.path().join("last_exit").exists(), "the next run starts without a record");
}

#[test]
fn a_controlled_exit_record_holds_the_status_and_reason() {
    let dir = tempfile::tempdir().unwrap();
    write_last_exit(dir.path().to_str().unwrap(), 3, "recovery chord");
    assert_eq!(fs::read_to_string(dir.path().join("last_exit")).unwrap(), "3 recovery chord\n");
}

#[test]
fn a_failed_controlled_exit_record_is_reported_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    assert!(try_write_last_exit(missing.to_str().unwrap(), 0, "quit").is_err());
}
