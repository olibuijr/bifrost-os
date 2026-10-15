//! Parity sdk-os-05: headless tests for the app-facing diagnostic reports and the archive module. Archives are
//! written to and read from temporary folders only.

use bifrost_app::archive;
use bifrost_app::diagnostic_report::{longest_stall, DiagnosticKind, DiagnosticReport};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// Diagnostic reports (diagnostic_report)

const LOG: &str = "[1700000000 12:00:00] thread 'main': panic: index out of bounds at src/shell/mod.rs:812:9\n\
[1700000100 12:01:40] thread 'watchdog': ui loop stalled for 21s\n\
not a record\n\
[1700000200 12:03:20] thread 'watchdog': ui loop stalled for 35s\n";

#[test]
fn headless_a_panic_line_is_read_as_a_crash_record_with_its_time() {
    let reports = DiagnosticReport::from_crash_log(LOG);
    assert_eq!(reports[0].unix, 1_700_000_000);
    assert_eq!(reports[0].kind, DiagnosticKind::Crash);
    assert!(reports[0].signature.contains("index out of bounds"));
}

#[test]
fn headless_a_stall_line_is_read_as_a_stall_with_its_whole_seconds() {
    let reports = DiagnosticReport::from_crash_log(LOG);
    assert_eq!(reports[1].kind, DiagnosticKind::Stall { seconds: 21 });
    assert_eq!(reports[1].signature, "ui loop stalled for 21s");
}

#[test]
fn headless_lines_that_are_not_records_are_skipped_and_order_is_kept() {
    let reports = DiagnosticReport::from_crash_log(LOG);
    assert_eq!(reports.len(), 3);
    let times: Vec<i64> = reports.iter().map(|r| r.unix).collect();
    assert_eq!(times, vec![1_700_000_000, 1_700_000_100, 1_700_000_200]);
}

#[test]
fn headless_the_longest_stall_is_the_largest_stall_in_the_log() {
    let reports = DiagnosticReport::from_crash_log(LOG);
    assert_eq!(longest_stall(&reports), Some(35));
}

#[test]
fn headless_a_log_without_a_stall_has_no_longest_stall() {
    let reports = DiagnosticReport::from_crash_log("[1700000000 12:00:00] thread 'main': panic: boom at src/x.rs:1:1\n");
    assert_eq!(longest_stall(&reports), None);
    assert_eq!(longest_stall(&[]), None);
}

// Archives (archive)

/// The first file named `name` under `dir`, searched depth first.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

#[test]
fn headless_a_folder_round_trips_through_an_archive_with_its_nested_files() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("notes");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), "first").unwrap();
    fs::write(src.join("sub").join("b.txt"), "second").unwrap();
    let packed = work.path().join("packed");
    fs::create_dir_all(&packed).unwrap();
    let zip = archive::compress_directory(&src, &packed, &mut |_, _| true).unwrap();
    assert!(zip.exists());
    let unpacked = work.path().join("unpacked");
    fs::create_dir_all(&unpacked).unwrap();
    let out = archive::expand(&zip, &unpacked, &mut |_, _| true).unwrap();
    assert!(out.skipped.is_empty());
    assert_eq!(fs::read_to_string(find(&out.dir, "a.txt").unwrap()).unwrap(), "first");
    assert_eq!(fs::read_to_string(find(&out.dir, "b.txt").unwrap()).unwrap(), "second");
}

#[test]
fn headless_a_single_file_is_archived_and_expanded_with_its_content() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("memo.txt");
    fs::write(&src, "one file").unwrap();
    let packed = work.path().join("packed");
    fs::create_dir_all(&packed).unwrap();
    let zip = archive::compress_file(&src, &packed, &mut |_, _| true).unwrap();
    let unpacked = work.path().join("unpacked");
    fs::create_dir_all(&unpacked).unwrap();
    let out = archive::expand(&zip, &unpacked, &mut |_, _| true).unwrap();
    assert_eq!(fs::read_to_string(find(&out.dir, "memo.txt").unwrap()).unwrap(), "one file");
}

#[test]
fn headless_a_folder_archive_refuses_a_file_and_a_file_archive_refuses_a_folder() {
    let work = tempfile::tempdir().unwrap();
    let file = work.path().join("memo.txt");
    fs::write(&file, "x").unwrap();
    let folder = work.path().join("folder");
    fs::create_dir_all(&folder).unwrap();
    let kind = |r: io::Result<PathBuf>| r.unwrap_err().kind();
    assert_eq!(kind(archive::compress_directory(&file, work.path(), &mut |_, _| true)), io::ErrorKind::InvalidInput);
    assert_eq!(kind(archive::compress_file(&folder, work.path(), &mut |_, _| true)), io::ErrorKind::InvalidInput);
}
