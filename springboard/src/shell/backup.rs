//! Backup and restore of the shell's own data (Notes, Contacts, Calendar,
//! Reminders, settings, notifications) as one archive in a small format of
//! our own, written and read in pure Rust.
//!
//! Layout: `SBBK1\n`, entry count (u32 BE), then per entry the name length
//! (u16 BE), the name (path relative to the data directory), the data length
//! (u64 BE), the data, and SHA-256 over name and data; the archive ends with
//! SHA-256 over everything before it. A restore verifies every checksum and
//! name before it touches a file, then writes each file through a temporary
//! name and a rename.
//!
//! A restore needs the shell stopped (it holds the data in memory and writes
//! it back), so it is staged as `restore.pending` and applied by
//! `apply_pending` at the start of the next run.

use super::sha256::{digest, Sha256};
use super::prefs::write_atomic;
use std::io;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"SBBK1\n";
/// Directories and files (relative to the data directory) a backup holds.
pub const SOURCES: [&str; 6] = ["Notes", "Contacts", "Calendar", "reminders.tsv", "settings.ini", "notifications.ini"];
pub const BACKUP_DIR: &str = "backups";
pub const PENDING: &str = "restore.pending";
const MAX_ENTRY: u64 = 256 * 1024 * 1024;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// A name that stays inside one of the sources.
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && name.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
        && SOURCES.iter().any(|s| name == *s || name.strip_prefix(s).is_some_and(|r| r.starts_with('/')))
}

/// Serialises `entries` (name, data) into an archive.
pub fn encode(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend((entries.len() as u32).to_be_bytes());
    for (name, data) in entries {
        out.extend((name.len() as u16).to_be_bytes());
        out.extend(name.as_bytes());
        out.extend((data.len() as u64).to_be_bytes());
        out.extend(data);
        let mut h = Sha256::new();
        h.update(name.as_bytes());
        h.update(data);
        out.extend(h.finish());
    }
    let sum = digest(&out);
    out.extend(sum);
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(bad("archive is truncated"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
}

/// Parses and fully verifies an archive.
pub fn decode(bytes: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
    if bytes.len() < MAGIC.len() + 4 + 32 || &bytes[..MAGIC.len()] != MAGIC {
        return Err(bad("not a backup archive"));
    }
    let (body, sum) = bytes.split_at(bytes.len() - 32);
    if digest(body)[..] != *sum {
        return Err(bad("archive checksum mismatch"));
    }
    let mut r = Reader(&body[MAGIC.len()..]);
    let count = u32::from_be_bytes(r.take(4)?.try_into().unwrap());
    let mut entries = Vec::new();
    for _ in 0..count {
        let nl = u16::from_be_bytes(r.take(2)?.try_into().unwrap()) as usize;
        let name = std::str::from_utf8(r.take(nl)?).map_err(|_| bad("bad name"))?.to_string();
        let dl = u64::from_be_bytes(r.take(8)?.try_into().unwrap());
        if dl > MAX_ENTRY {
            return Err(bad("entry too large"));
        }
        let data = r.take(dl as usize)?.to_vec();
        let check = r.take(32)?;
        let mut h = Sha256::new();
        h.update(name.as_bytes());
        h.update(&data);
        if h.finish()[..] != *check {
            return Err(bad("entry checksum mismatch"));
        }
        if !name_ok(&name) {
            return Err(bad("unsafe entry name"));
        }
        entries.push((name, data));
    }
    if !r.0.is_empty() {
        return Err(bad("trailing bytes"));
    }
    Ok(entries)
}

fn walk(base: &Path, rel: &str, out: &mut Vec<String>, depth: u32) {
    let path = base.join(rel);
    if path.is_file() {
        out.push(rel.to_string());
    } else if path.is_dir() && depth < 4 {
        let mut names: Vec<String> = std::fs::read_dir(&path).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
        names.sort();
        for n in names {
            walk(base, &format!("{rel}/{n}"), out, depth + 1);
        }
    }
}

/// The files of every source that exists under `dir`, sorted.
pub fn collect(dir: &Path) -> Vec<String> {
    let mut v = vec![];
    for s in SOURCES {
        walk(dir, s, &mut v, 0);
    }
    v
}

/// Writes an archive of `dir`'s data to `out` (via a temporary name); returns the file count.
pub fn create(dir: &Path, out: &Path) -> io::Result<usize> {
    let mut entries = vec![];
    for name in collect(dir) {
        if name_ok(&name) {
            entries.push((name.clone(), std::fs::read(dir.join(&name))?));
        }
    }
    write_atomic(out, &encode(&entries))?;
    Ok(entries.len())
}

/// Verifies `archive`, then writes its files under `dir`; returns the file count.
pub fn restore(dir: &Path, archive: &Path) -> io::Result<usize> {
    let entries = decode(&std::fs::read(archive)?)?;
    for (name, data) in &entries {
        write_atomic(&dir.join(name), data)?;
    }
    Ok(entries.len())
}

/// `<dir>/backups/backup-<unix>.sbk`
pub fn backup_path(dir: &Path, unix: u64) -> PathBuf {
    dir.join(BACKUP_DIR).join(format!("backup-{unix}.sbk"))
}

/// Backs up now; returns the file and how many files it holds.
pub fn back_up_now(dir: &Path) -> io::Result<(PathBuf, usize)> {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = backup_path(dir, unix);
    let n = create(dir, &path)?;
    Ok((path, n))
}

/// The newest backup in `dir/backups` (by the number in its name) and its unix time.
pub fn latest(dir: &Path) -> Option<(PathBuf, u64)> {
    std::fs::read_dir(dir.join(BACKUP_DIR))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let t = name.strip_prefix("backup-")?.strip_suffix(".sbk")?.parse::<u64>().ok()?;
            Some((e.path(), t))
        })
        .max_by_key(|&(_, t)| t)
}

/// Stages `archive` for the next start after checking it is sound.
pub fn stage(dir: &Path, archive: &Path) -> io::Result<usize> {
    let bytes = std::fs::read(archive)?;
    let n = decode(&bytes)?.len();
    write_atomic(&dir.join(PENDING), &bytes)?;
    Ok(n)
}

/// Applies a staged restore (at start-up, before anything loads); the
/// staged file is removed either way so a bad one cannot loop.
pub fn apply_pending(dir: &Path) -> Option<io::Result<usize>> {
    let p = dir.join(PENDING);
    if !p.exists() {
        return None;
    }
    let r = restore(dir, &p);
    let _ = std::fs::remove_file(&p);
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trip_and_tamper_detection() {
        let e = vec![("Notes/1.md".to_string(), b"hello".to_vec()), ("settings.ini".to_string(), vec![])];
        let a = encode(&e);
        assert_eq!(decode(&a).unwrap(), e);
        for i in [0, 8, 20, a.len() - 1] {
            let mut t = a.clone();
            t[i] ^= 1;
            assert!(decode(&t).is_err(), "flip at {i}");
        }
        assert!(decode(&a[..a.len() - 5]).is_err());
    }

    #[test]
    fn names_stay_inside_sources() {
        assert!(name_ok("Notes/a/b.md") && name_ok("reminders.tsv"));
        for n in ["", "/etc/passwd", "Notes/../x", "Notes//x", "other.txt", "NotesX/a", "Notes\\a"] {
            assert!(!name_ok(n), "{n}");
        }
        assert!(decode(&encode(&[("../evil".into(), vec![1])])).is_err());
    }

    #[test]
    fn create_then_restore_into_a_fresh_dir() {
        let src = scratch("src");
        std::fs::create_dir_all(src.join("Notes")).unwrap();
        std::fs::write(src.join("Notes/1.md"), "note").unwrap();
        std::fs::write(src.join("settings.ini"), "a=b\n").unwrap();
        std::fs::write(src.join("unrelated.bin"), "x").unwrap();
        let out = src.join("out.sbk");
        assert_eq!(create(&src, &out).unwrap(), 2);
        let dst = scratch("dst");
        assert_eq!(restore(&dst, &out).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(dst.join("Notes/1.md")).unwrap(), "note");
        assert!(!dst.join("unrelated.bin").exists());
    }

    #[test]
    fn staged_restore_applies_once_and_bad_ones_change_nothing() {
        let src = scratch("stage");
        std::fs::write(src.join("settings.ini"), "v=1\n").unwrap();
        let out = src.join("b.sbk");
        create(&src, &out).unwrap();
        std::fs::write(src.join("settings.ini"), "v=2\n").unwrap();
        stage(&src, &out).unwrap();
        assert_eq!(apply_pending(&src).unwrap().unwrap(), 1);
        assert_eq!(std::fs::read_to_string(src.join("settings.ini")).unwrap(), "v=1\n");
        assert!(apply_pending(&src).is_none());
        std::fs::write(src.join(PENDING), b"garbage").unwrap();
        assert!(apply_pending(&src).unwrap().is_err());
        assert!(!src.join(PENDING).exists());
        assert_eq!(std::fs::read_to_string(src.join("settings.ini")).unwrap(), "v=1\n");
    }

    #[test]
    fn latest_picks_the_highest_stamp() {
        let d = scratch("latest");
        std::fs::create_dir_all(d.join(BACKUP_DIR)).unwrap();
        for t in [5, 100, 20] {
            std::fs::write(backup_path(&d, t), b"x").unwrap();
        }
        assert_eq!(latest(&d).unwrap().1, 100);
    }
}
