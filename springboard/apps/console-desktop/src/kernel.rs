//! Kernel Reports: what the kernel left behind about a crash. The records the persistent store kept across a reboot
//! (`/sys/fs/pstore`, where a panic or oops is written) and the previous boot's kernel warnings and errors from the
//! journal. Each pstore file is one row whose detail pane shows the file.

use crate::model::{self, Level, LogEntry};
use crate::source::Source;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the kernel's persistent store is mounted.
pub const PSTORE_DIR: &str = "/sys/fs/pstore";
/// The process and subsystem the pstore rows name (a record is not written by a process).
pub const PROCESS: &str = "pstore";
pub const SUBSYSTEM: &str = "kernel";
/// The footer note while the source is shown.
pub const NOTE: &str = "Kernel crash records from the persistent store, and the previous boot's kernel warnings and errors.";
/// Most bytes of a record the detail pane reads.
const DETAIL_MAX: u64 = 64 << 10;

/// "512 bytes", "3.2 KB", "1.5 MB".
pub fn size_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// The row for a pstore file: a fault, dated by the file's modification time (microseconds), with the file name as the
/// category so the detail pane can find the file again.
pub fn pstore_entry(file: &str, size: u64, modified_us: i64) -> LogEntry {
    LogEntry {
        date: modified_us,
        process: PROCESS.into(),
        pid: 0,
        subsystem: SUBSYSTEM.into(),
        category: file.into(),
        level: Level::Fault,
        message: format!("Kernel crash record {file} ({})", size_text(size)),
    }
}

/// True when `journalctl`'s complaint means the journal has no previous boot, which is an empty list and not an error.
pub fn no_previous_boot(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    ["specified boot", "no such boot", "no journal files", "boot id"].iter().any(|m| lower.contains(m))
}

/// The pstore rows of directory `dir`, one per regular file.
pub fn pstore_rows(dir: &Path) -> Vec<LogEntry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read.filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
            Some(pstore_entry(&e.file_name().to_string_lossy(), meta.len(), modified.as_micros() as i64))
        })
        .collect()
}

/// A file name that stays in the directory: no separators and not a dot name.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// The kernel reports source.
pub struct KernelReports {
    pub pstore: PathBuf,
}

impl Default for KernelReports {
    fn default() -> Self {
        KernelReports { pstore: PathBuf::from(PSTORE_DIR) }
    }
}

impl Source for KernelReports {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        if bifrost_app::testing::pinned() {
            return Ok(Vec::new());
        }
        let mut rows = pstore_rows(&self.pstore);
        let n = limit.to_string();
        let out = Command::new("journalctl")
            .args(["-k", "-b", "-1", "-p", "warning", "-o", "json", "--no-pager", "-n", n.as_str()])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("journalctl: {e}"))?;
        if out.status.success() {
            rows.extend(String::from_utf8_lossy(&out.stdout).lines().filter_map(model::parse_journal_line));
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if rows.is_empty() && !err.is_empty() && !no_previous_boot(&err) {
                return Err(err);
            }
        }
        rows.sort_by_key(|e| e.date);
        if rows.len() > limit {
            rows.drain(..rows.len() - limit);
        }
        Ok(rows)
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        use std::io::Read;
        if e.process != PROCESS || e.subsystem != SUBSYSTEM || !plain_name(&e.category) {
            return None;
        }
        let file = std::fs::File::open(self.pstore.join(&e.category)).ok()?;
        let mut bytes = Vec::new();
        file.take(DETAIL_MAX).read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("console-kernel-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sizes_read_in_bytes_kilobytes_and_megabytes() {
        assert_eq!(size_text(0), "0 bytes");
        assert_eq!(size_text(1023), "1023 bytes");
        assert_eq!(size_text(3277), "3.2 KB");
        assert_eq!(size_text(3 << 19), "1.5 MB");
    }

    #[test]
    fn a_pstore_file_is_a_fault_row_that_names_the_file() {
        let e = pstore_entry("dmesg-ramoops-0", 2048, 5_000_000);
        assert_eq!(e.level, Level::Fault);
        assert_eq!((e.process.as_str(), e.subsystem.as_str(), e.category.as_str()), ("pstore", "kernel", "dmesg-ramoops-0"));
        assert_eq!(e.message, "Kernel crash record dmesg-ramoops-0 (2.0 KB)");
        assert_eq!(e.date, 5_000_000);
    }

    #[test]
    fn a_missing_previous_boot_is_not_an_error() {
        assert!(no_previous_boot("Data from the specified boot (-1) is not available: No such boot ID in journal"));
        assert!(no_previous_boot("No journal files were found."));
        assert!(!no_previous_boot("Failed to open journal: Permission denied"));
    }

    #[test]
    fn pstore_rows_list_regular_files_only() {
        let dir = scratch("rows");
        std::fs::write(dir.join("dmesg-ramoops-0"), "Kernel panic - not syncing").unwrap();
        std::fs::write(dir.join("console-ramoops-0"), "boot log").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();
        let mut rows = pstore_rows(&dir);
        rows.sort_by(|a, b| a.category.cmp(&b.category));
        assert_eq!(rows.iter().map(|r| r.category.as_str()).collect::<Vec<_>>(), vec!["console-ramoops-0", "dmesg-ramoops-0"]);
        assert!(rows.iter().all(|r| r.date > 0));
        assert!(pstore_rows(&dir.join("absent")).is_empty(), "no pstore mounted is an empty list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_detail_is_the_record_and_never_a_file_outside_the_store() {
        let dir = scratch("detail");
        std::fs::write(dir.join("dmesg-ramoops-0"), "Oops: 0002\nRIP: 0010:foo").unwrap();
        std::fs::write(dir.parent().unwrap().join(format!("console-kernel-secret-{}", std::process::id())), "secret").unwrap();
        let mut source = KernelReports { pstore: dir.clone() };
        let row = pstore_entry("dmesg-ramoops-0", 24, 1);
        assert_eq!(source.detail(&row).as_deref(), Some("Oops: 0002\nRIP: 0010:foo"));
        let escape = pstore_entry(&format!("../console-kernel-secret-{}", std::process::id()), 6, 1);
        assert_eq!(source.detail(&escape), None);
        let journal_row = LogEntry { process: "kernel".into(), message: "BUG".into(), ..LogEntry::default() };
        assert_eq!(source.detail(&journal_row), None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(dir.parent().unwrap().join(format!("console-kernel-secret-{}", std::process::id())));
    }
}
