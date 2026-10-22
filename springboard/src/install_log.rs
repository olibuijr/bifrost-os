//! Install failure log: one tab-separated line per failed install (a system update from `updated`, an app from the
//! store), kept in the shell data directory so the diagnostics page can list it. The file rotates to `install-failures.log.1`
//! once it passes `MAX_BYTES`, so it never grows without bound.

use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE: &str = "install-failures.log";
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub unix: i64,
    /// what failed, such as "system update"
    pub what: String,
    pub message: String,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// One record. Tabs and line breaks inside the fields become spaces so each record stays one line.
pub fn line(unix: i64, what: &str, message: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect::<String>();
    format!("{unix}\t{}\t{}\n", clean(what), clean(message))
}

pub fn parse(text: &str) -> Vec<Failure> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let unix = it.next()?.parse().ok()?;
            let what = it.next()?.to_string();
            let message = it.next()?.to_string();
            Some(Failure { unix, what, message })
        })
        .collect()
}

/// Appends a failure, rotating the file first when it is over the size limit.
pub fn append(dir: &Path, unix: i64, what: &str, message: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = path(dir);
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_BYTES) {
        std::fs::rename(&p, dir.join(format!("{FILE}.1")))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
    f.write_all(line(unix, what, message).as_bytes())
}

/// Logs a failure from a worker thread. Write errors are dropped: a failed install must not fail again because its
/// log could not be written.
pub fn record(dir: &Path, what: &str, message: &str) {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let _ = append(dir, unix, what, message);
}

/// Logs a failure unless the newest record already says the same thing. A background check that fails the same way
/// every 15 minutes (offline, a bad package) then leaves one record, not one per check.
pub fn record_once(dir: &Path, what: &str, message: &str) {
    let clean = |s: &str| s.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect::<String>();
    if read(dir).last().is_some_and(|f| f.what == clean(what) && f.message == clean(message)) {
        return;
    }
    record(dir, what, message);
}

/// Passes a result through and logs its error as `subject: error` (once, see `record_once`). For the install paths
/// that run in the background and drop the error: the privd daemon and the OTA checker.
pub fn logged<T, E: std::fmt::Display>(dir: &Path, what: &str, subject: &str, result: Result<T, E>) -> Result<T, E> {
    if let Err(e) = &result {
        record_once(dir, what, &format!("{subject}: {e}"));
    }
    result
}

/// Every recorded failure, oldest first. A missing file is an empty log.
pub fn read(dir: &Path) -> Vec<Failure> {
    std::fs::read_to_string(path(dir)).map(|t| parse(&t)).unwrap_or_default()
}

/// The newest `max` failures, newest first, for the diagnostics page.
pub fn recent(dir: &Path, max: usize) -> Vec<Failure> {
    let mut all = read(dir);
    all.reverse();
    all.truncate(max);
    all
}

/// The newest `max` failures across several data directories (the user's and the system one the root daemons write to),
/// newest first. A directory that cannot be read counts as empty.
pub fn recent_in(dirs: &[&Path], max: usize) -> Vec<Failure> {
    // a shell that runs as root has the same directory twice
    let mut seen: Vec<&Path> = vec![];
    let mut all: Vec<Failure> = vec![];
    for d in dirs {
        if !seen.contains(d) {
            seen.push(*d);
            all.extend(read(d));
        }
    }
    all.sort_by(|a, b| b.unix.cmp(&a.unix));
    all.truncate(max);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("updated-installlog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn line_keeps_one_record_per_line() {
        let l = line(42, "system\tupdate", "pacman\nfailed\r now");
        assert_eq!(l.matches('\n').count(), 1);
        assert_eq!(l, "42\tsystem update\tpacman failed  now\n");
    }

    #[test]
    fn parse_reads_back_what_line_wrote() {
        let text = format!("{}{}", line(1, "system update", "offline"), line(2, "system update", "disk full"));
        let got = parse(&text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], Failure { unix: 2, what: "system update".into(), message: "disk full".into() });
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let got = parse("not a record\nx\ty\tz\n7\tsystem update\tok\n");
        assert_eq!(got, vec![Failure { unix: 7, what: "system update".into(), message: "ok".into() }]);
    }

    #[test]
    fn append_then_recent_is_newest_first() {
        let d = tmp("recent");
        assert!(read(&d).is_empty());
        append(&d, 10, "system update", "first").unwrap();
        append(&d, 20, "system update", "second").unwrap();
        append(&d, 30, "system update", "third").unwrap();
        let newest = recent(&d, 2);
        assert_eq!(newest.iter().map(|f| f.message.as_str()).collect::<Vec<_>>(), ["third", "second"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn record_once_collapses_a_repeated_failure_but_keeps_a_new_one() {
        let d = tmp("once");
        record_once(&d, "system package", "keyboard: offline");
        record_once(&d, "system package", "keyboard: offline");
        record_once(&d, "system package", "keyboard: disk full");
        record_once(&d, "system package", "keyboard: offline");
        let got = read(&d);
        assert_eq!(got.iter().map(|f| f.message.as_str()).collect::<Vec<_>>(), ["keyboard: offline", "keyboard: disk full", "keyboard: offline"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn logged_passes_both_results_through_and_logs_only_the_error() {
        let d = tmp("logged");
        assert_eq!(logged(&d, "app install", "notes", Ok::<_, String>(3)), Ok(3));
        assert!(read(&d).is_empty());
        assert_eq!(logged(&d, "app install", "notes", Err::<u8, _>("bad hash".to_string())), Err("bad hash".to_string()));
        let got = read(&d);
        assert_eq!((got[0].what.as_str(), got[0].message.as_str()), ("app install", "notes: bad hash"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recent_in_merges_directories_newest_first_and_skips_a_missing_one() {
        let (a, b) = (tmp("merge-a"), tmp("merge-b"));
        append(&a, 10, "app install", "a1").unwrap();
        append(&a, 30, "app install", "a2").unwrap();
        append(&b, 20, "package install", "b1").unwrap();
        let missing = std::env::temp_dir().join(format!("updated-installlog-none-{}", std::process::id()));
        let got = recent_in(&[&a, &missing, &b], 2);
        assert_eq!(got.iter().map(|f| f.message.as_str()).collect::<Vec<_>>(), ["a2", "b1"]);
        assert_eq!(recent_in(&[&a, &a], 10).len(), 2, "the same directory is read once");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn oversized_log_rotates_before_append() {
        let d = tmp("rotate");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(path(&d), vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        append(&d, 5, "system update", "after rotation").unwrap();
        assert!(d.join(format!("{FILE}.1")).exists());
        let now = read(&d);
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].message, "after rotation");
        let _ = std::fs::remove_dir_all(&d);
    }
}
