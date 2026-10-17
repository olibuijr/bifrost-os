//! Spin Reports: the samples the shell writes when its UI loop keeps running but stops responding to input for a while.
//! A spin report is a plain-text file with the `.spin` extension in the `spin` folder of a shell data directory (the
//! system's and the user's, as Hang Diagnostics reads them). Each file is one row, dated by its modification time, and
//! its detail pane shows the file.
//!
//! The shell does not write these files yet, so the list is empty until it does; the reader and the listing are ready
//! for them. The pure parts (the name rule, the row, the file reader) are tested without a system.

use crate::diagnostics::{self, PROCESS};
use crate::model::{Level, LogEntry};
use crate::source::Source;
use std::path::{Path, PathBuf};

/// The folder under a shell data directory where spin reports are written.
pub const SPIN_DIR: &str = "spin";
/// The extension of a spin report file.
pub const EXTENSION: &str = "spin";
/// The subsystem the spin rows name.
pub const SUBSYSTEM: &str = "spin";
/// The footer note while the source is shown.
pub const NOTE: &str = "Spin reports: samples taken while the shell's UI loop kept running but stopped responding to input.";
/// Most bytes of a report the detail pane reads.
const DETAIL_MAX: u64 = 64 << 10;

/// True for a file name that is a spin report: a plain name (no separators, not a dot name) ending in `.spin`.
pub fn is_spin_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && name.len() > EXTENSION.len() + 1
        && name.ends_with(&format!(".{EXTENSION}"))
}

/// The row for a spin report file: a fault, dated by the file's modification time (microseconds), with the file name as
/// the category so the detail pane can find the file again.
pub fn spin_entry(file: &str, size: u64, modified_us: i64) -> LogEntry {
    LogEntry {
        date: modified_us,
        process: PROCESS.into(),
        pid: 0,
        subsystem: SUBSYSTEM.into(),
        category: file.into(),
        level: Level::Fault,
        message: format!("Spin report {file} ({})", crate::kernel::size_text(size)),
        invocation: String::new(),
    }
}

/// The spin rows of one directory, one per spin report file, oldest first.
pub fn spin_rows(dir: &Path) -> Vec<LogEntry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut rows: Vec<LogEntry> = read
        .filter_map(|e| e.ok())
        .filter(|e| is_spin_name(&e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
            Some(spin_entry(&e.file_name().to_string_lossy(), meta.len(), modified.as_micros() as i64))
        })
        .collect();
    rows.sort_by_key(|e| e.date);
    rows
}

/// The text of the spin report `name` in `dir`, at most `DETAIL_MAX` bytes. None for a name that is not a spin report or a
/// file that cannot be read.
pub fn read_report(dir: &Path, name: &str) -> Option<String> {
    use std::io::Read;
    if !is_spin_name(name) {
        return None;
    }
    let file = std::fs::File::open(dir.join(name)).ok()?;
    let mut bytes = Vec::new();
    file.take(DETAIL_MAX).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The spin folders the shell may write to: one under each of the crash-log directories.
pub fn spin_dirs() -> Vec<PathBuf> {
    diagnostics::crash_log_dirs().into_iter().map(|d| d.join(SPIN_DIR)).collect()
}

/// The spin reports source.
pub struct SpinReports {
    pub dirs: Vec<PathBuf>,
}

impl Default for SpinReports {
    fn default() -> Self {
        SpinReports { dirs: spin_dirs() }
    }
}

impl Source for SpinReports {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        if bifrost_app::testing::pinned() {
            return Ok(Vec::new());
        }
        let mut rows: Vec<LogEntry> = self.dirs.iter().flat_map(|d| spin_rows(d)).collect();
        rows.sort_by_key(|e| e.date);
        if rows.len() > limit {
            rows.drain(..rows.len() - limit);
        }
        Ok(rows)
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        if e.process != PROCESS || e.subsystem != SUBSYSTEM {
            return None;
        }
        self.dirs.iter().find_map(|d| read_report(d, &e.category))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("console-spin-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_plain_names_ending_in_spin_are_reports() {
        assert!(is_spin_name("springboard-2026-10-17.spin"));
        assert!(!is_spin_name("crash.log"));
        assert!(!is_spin_name(".spin"));
        assert!(!is_spin_name("spin"));
        assert!(!is_spin_name("../x.spin"));
        assert!(!is_spin_name("a/b.spin"));
    }

    #[test]
    fn a_spin_row_is_a_fault_named_by_its_file() {
        let e = spin_entry("a.spin", 2048, 5);
        assert_eq!(e.category, "a.spin");
        assert_eq!(e.subsystem, SUBSYSTEM);
        assert_eq!(e.level, Level::Fault);
        assert_eq!(e.date, 5);
        assert_eq!(e.message, "Spin report a.spin (2.0 KB)");
    }

    #[test]
    fn rows_list_only_spin_files_and_the_reader_returns_their_text() {
        let dir = scratch("rows");
        std::fs::write(dir.join("one.spin"), "frame a\nframe b\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a report").unwrap();
        let rows = spin_rows(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, "one.spin");
        assert_eq!(read_report(&dir, "one.spin").as_deref(), Some("frame a\nframe b\n"));
        assert_eq!(read_report(&dir, "notes.txt"), None, "a file that is not a spin report is not read");
        assert_eq!(read_report(&dir, "missing.spin"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_has_no_rows() {
        assert!(spin_rows(Path::new("/nonexistent/bifrost-spin-test")).is_empty());
    }

    #[test]
    fn the_source_reads_a_row_from_the_first_folder_that_has_it() {
        let a = scratch("first");
        let b = scratch("second");
        std::fs::write(b.join("late.spin"), "sample").unwrap();
        let mut src = SpinReports { dirs: vec![a.clone(), b.clone()] };
        let rows = src.load(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(src.detail(&rows[0]).as_deref(), Some("sample"));
        let other = LogEntry { category: "x".into(), ..rows[0].clone() };
        assert_eq!(src.detail(&other), None);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
