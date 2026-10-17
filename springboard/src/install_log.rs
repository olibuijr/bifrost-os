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
