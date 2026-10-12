//! Kernel crash records in pstore (`/sys/fs/pstore`). The kernel writes a kernel-message dump there when it panics or
//! oopses, and the record survives the reboot. The early-userspace supervisor (`bootweave/supervisor.rs`) reads them on
//! the next boot, shows "Your computer restarted because of a problem", and removes them once the screen is up, so the
//! same record never shows twice. Panic and oops dumps are the `dmesg-<backend>-<id>` files; console and pmsg records
//! are not crash reports and are left alone. Kernel documentation: `Documentation/admin-guide/ramoops.rst` and
//! `Documentation/admin-guide/pstore-blk.rst`.

use std::path::{Path, PathBuf};

pub const DIR: &str = "/sys/fs/pstore";

/// Whether a pstore file name is a panic or oops dump: `dmesg-` followed by the backend and record id.
pub fn is_crash_record(name: &str) -> bool {
    name.strip_prefix("dmesg-").is_some_and(|rest| !rest.is_empty())
}

/// The crash-record names among `names`, in name order. The sort is lexicographic (`-10` comes before `-2`), so the
/// order fixes the list only; it does not give the age of the records.
pub fn select_crash_records<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut found: Vec<&str> = names.into_iter().filter(|n| is_crash_record(n)).collect();
    found.sort_unstable();
    found
}

/// The crash records in `dir` (empty when pstore is absent or unreadable).
pub fn crash_records(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let names: Vec<String> = entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    select_crash_records(names.iter().map(String::as_str)).into_iter().map(|n| dir.join(n)).collect()
}

/// Removes records once they have been shown. A record the kernel refuses to remove stays for the next boot.
pub fn clear(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_panic_and_oops_dumps_are_crash_records() {
        assert!(is_crash_record("dmesg-efi-1700000000000000"));
        assert!(is_crash_record("dmesg-ramoops-0"));
        assert!(!is_crash_record("console-ramoops-0"));
        assert!(!is_crash_record("pmsg-ramoops-0"));
        assert!(!is_crash_record("ftrace-ramoops-0"));
    }

    #[test]
    fn an_empty_id_is_not_a_record() {
        assert!(!is_crash_record("dmesg-"));
        assert!(!is_crash_record("dmesg"));
        assert!(!is_crash_record("xdmesg-efi-1"));
    }

    #[test]
    fn records_come_out_sorted_and_filtered() {
        let names = ["pmsg-ramoops-0", "dmesg-ramoops-2", "console-ramoops-0", "dmesg-ramoops-10", "dmesg-ramoops-9"];
        assert_eq!(select_crash_records(names), vec!["dmesg-ramoops-10", "dmesg-ramoops-2", "dmesg-ramoops-9"]);
    }

    #[test]
    fn no_records_when_the_list_is_empty() {
        assert!(select_crash_records(Vec::<&str>::new()).is_empty());
        assert!(select_crash_records(["console-ramoops-0"]).is_empty());
    }

    #[test]
    fn a_missing_pstore_directory_has_no_records() {
        assert!(crash_records(Path::new("/nonexistent/bifrost/pstore")).is_empty());
    }

    #[test]
    fn records_in_a_directory_are_joined_to_it() {
        let dir = std::env::temp_dir().join(format!("bifrost-pstore-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dmesg-efi-7"), b"x").unwrap();
        std::fs::write(dir.join("console-ramoops-0"), b"x").unwrap();
        let found = crash_records(&dir);
        assert_eq!(found, vec![dir.join("dmesg-efi-7")]);
        clear(&found);
        assert!(crash_records(&dir).is_empty());
        assert!(dir.join("console-ramoops-0").exists(), "non-crash records are kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
