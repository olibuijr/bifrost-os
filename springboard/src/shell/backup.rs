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

//!
//! Encryption (format version 2, `SBBK2\n`): the version 1 bytes above are sealed with XChaCha20-Poly1305 under a
//! random 256-bit key kept in the shell keychain (`backup_key`), with a random 24-byte nonce in the header. The header
//! is bound as associated data. Version 1 archives stay readable without a key.

use super::sha256::{digest, Sha256};
use super::prefs::write_atomic;
use crate::keychain::Store;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use std::io;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"SBBK1\n";
const MAGIC_V2: &[u8] = b"SBBK2\n";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// The keychain app id (`<data dir>/keychain/bifrost.backup.kc`) and item that hold the archive key.
pub const KEY_APP: &str = "bifrost.backup";
const KEY_ACCOUNT: &str = "archive-key";
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
        if !name_ok(&name) && !super::migration::app_data_name(&name) {
            return Err(bad("unsafe entry name"));
        }
        entries.push((name, data));
    }
    if !r.0.is_empty() {
        return Err(bad("trailing bytes"));
    }
    Ok(entries)
}

pub fn walk(base: &Path, rel: &str, out: &mut Vec<String>, depth: u32) {
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

/// The file, in the data folder, that holds the items the backup leaves out (see [`exclusions`]).
pub const EXCLUSIONS_FILE: &str = "backup-exclusions.txt";

/// The items excluded from backup, read from `dir/backup-exclusions.txt`. A missing file excludes nothing.
pub fn exclusions(dir: &Path) -> bifrost_core::backup_exclusion::ExclusionSet {
    let text = std::fs::read_to_string(dir.join(EXCLUSIONS_FILE)).unwrap_or_default();
    bifrost_core::backup_exclusion::from_text(&text)
}

/// Writes the exclusion set to `dir/backup-exclusions.txt`.
pub fn save_exclusions(dir: &Path, set: &bifrost_core::backup_exclusion::ExclusionSet) -> io::Result<()> {
    write_atomic(&dir.join(EXCLUSIONS_FILE), bifrost_core::backup_exclusion::to_text(set))
}

/// Whether the file `rel` (relative to `dir`) is excluded: it or a folder above it is an excluded item. The model
/// reports each item on its own, so the folders on the way down are checked here.
pub fn excluded_under(set: &bifrost_core::backup_exclusion::ExclusionSet, dir: &Path, rel: &str) -> bool {
    let mut path = dir.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
        if set.is_excluded(&path.to_string_lossy()).unwrap_or(false) {
            return true;
        }
    }
    false
}

/// The unix times of the backups in `dir/backups`, from their names, in no particular order.
pub fn archive_times(dir: &Path) -> Vec<u64> {
    std::fs::read_dir(dir.join(BACKUP_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_prefix("backup-")?.strip_suffix(".sbk")?.parse::<u64>().ok()
        })
        .collect()
}

/// The files of every source that exists under `dir`, sorted.
pub fn collect(dir: &Path) -> Vec<String> {
    let mut v = vec![];
    for s in SOURCES {
        walk(dir, s, &mut v, 0);
    }
    v
}

/// The archive key: read from the keychain, or created on first use from `/dev/urandom`.
pub fn backup_key(dir: &Path) -> io::Result<[u8; 32]> {
    let store = Store::open(dir, KEY_APP).map_err(|e| io::Error::other(e.to_string()))?;
    match store.get(KEY_APP, KEY_ACCOUNT) {
        Ok(bytes) => bytes.as_slice().try_into().map_err(|_| bad("the backup key in the keychain has the wrong length")),
        Err(crate::keychain::Error::ItemNotFound) => {
            let key: [u8; 32] = random_bytes()?;
            store.add(KEY_APP, KEY_ACCOUNT, &key).map_err(|e| io::Error::other(e.to_string()))?;
            Ok(key)
        }
        Err(e) => Err(io::Error::other(e.to_string())),
    }
}

fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    use std::io::Read;
    let mut buf = [0u8; N];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf))?;
    Ok(buf)
}

/// Seals a version 1 body as a version 2 archive under `key` with a fresh random nonce.
pub fn seal(key: &[u8; 32], body: &[u8]) -> io::Result<Vec<u8>> {
    let nonce: [u8; NONCE_LEN] = random_bytes()?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: body, aad: MAGIC_V2 })
        .map_err(|_| io::Error::other("backup encryption failed"))?;
    let mut out = Vec::with_capacity(MAGIC_V2.len() + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC_V2);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// The version 1 body of an archive's bytes. A version 1 archive is returned as it is (its own checksums are checked
/// by [`decode`]); a version 2 archive is opened with the key that `key` supplies, which is asked for only then.
pub fn open(bytes: &[u8], key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<Vec<u8>> {
    if !bytes.starts_with(MAGIC_V2) {
        // Only the version 1 header is plain; any other header (a damaged version 2 one included) is refused here.
        if bytes.starts_with(MAGIC) {
            return Ok(bytes.to_vec());
        }
        return Err(bad("not a backup archive"));
    }
    let rest = &bytes[MAGIC_V2.len()..];
    if rest.len() < NONCE_LEN + TAG_LEN {
        return Err(bad("encrypted backup is truncated"));
    }
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key()?));
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: MAGIC_V2 })
        .map_err(|_| bad("the backup key does not open this archive, or the archive was changed"))
}

/// Writes an archive of `dir`'s data to `out` (via a temporary name), sealed under `key`; returns the file count.
pub fn create_with(dir: &Path, out: &Path, key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<usize> {
    let mut entries = vec![];
    let excluded = exclusions(dir);
    for name in collect(dir) {
        if name_ok(&name) && !excluded_under(&excluded, dir, &name) {
            entries.push((name.clone(), std::fs::read(dir.join(&name))?));
        }
    }
    write_atomic(out, &seal(&key()?, &encode(&entries))?)?;
    Ok(entries.len())
}

/// Writes an encrypted archive of `dir`'s data to `out`, with the keychain's archive key; returns the file count.
pub fn create(dir: &Path, out: &Path) -> io::Result<usize> {
    create_with(dir, out, &|| backup_key(dir))
}

/// Writes the entries that `pick` accepts under `dir`, each through a temporary name; returns how many were written.
/// The picker is how a migration brings only some categories (see `shell::migration`).
pub fn write_entries(dir: &Path, entries: &[(String, Vec<u8>)], pick: &dyn Fn(&str) -> bool) -> io::Result<usize> {
    let mut n = 0;
    for (name, data) in entries.iter().filter(|(name, _)| pick(name)) {
        write_atomic(&dir.join(name), data)?;
        n += 1;
    }
    Ok(n)
}

/// Verifies `archive`, then writes its files under `dir`; returns the file count.
pub fn restore_with(dir: &Path, archive: &Path, key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<usize> {
    let entries = decode(&open(&std::fs::read(archive)?, key)?)?;
    write_entries(dir, &entries, &|_| true)
}

/// Restores `archive` into `dir`, with the keychain's archive key for a version 2 archive.
pub fn restore(dir: &Path, archive: &Path) -> io::Result<usize> {
    restore_with(dir, archive, &|| backup_key(dir))
}

/// `<dir>/backups/backup-<unix>.sbk`
pub fn backup_path(dir: &Path, unix: u64) -> PathBuf {
    dir.join(BACKUP_DIR).join(format!("backup-{unix}.sbk"))
}

/// Backs up now into the data folder; returns the file and how many files it holds.
pub fn back_up_now(dir: &Path) -> io::Result<(PathBuf, usize)> {
    back_up_to(dir, dir)
}

/// Backs up the data in `dir` into `dest/backups` (the chosen disk, Settings > Backup). The key stays with the data in
/// `dir`; only the archive goes to `dest`.
pub fn back_up_to(dir: &Path, dest: &Path) -> io::Result<(PathBuf, usize)> {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = backup_path(dest, unix);
    std::fs::create_dir_all(dest.join(BACKUP_DIR))?;
    let n = create(dir, &path)?;
    Ok((path, n))
}

/// The folder backups are kept in: the chosen destination, or the data folder when none is chosen (`chosen` empty).
pub fn destination_root(data_dir: &Path, chosen: &str) -> PathBuf {
    if chosen.is_empty() {
        data_dir.to_path_buf()
    } else {
        PathBuf::from(chosen)
    }
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

/// The backup diagnostics: how many archives are kept, the newest one's time and size, and whether the newest one
/// opens and decodes (its checksums and, for a version 2 archive, its seal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupStatus {
    pub count: usize,
    pub latest_unix: Option<u64>,
    pub latest_bytes: u64,
    /// `None` when there is no archive to check.
    pub verified: Option<bool>,
}

/// Reads the backup status under `dir/backups`, checking the newest archive with `key`.
pub fn status_with(dir: &Path, key: &dyn Fn() -> io::Result<[u8; 32]>) -> BackupStatus {
    let count = archive_times(dir).len();
    let Some((path, unix)) = latest(dir) else {
        return BackupStatus { count, latest_unix: None, latest_bytes: 0, verified: None };
    };
    let latest_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
    let verified = std::fs::read(&path).ok().map(|bytes| open(&bytes, key).and_then(|body| decode(&body)).is_ok());
    BackupStatus { count, latest_unix: Some(unix), latest_bytes, verified }
}

/// The backup status of `dir`, with the keychain's archive key.
pub fn status(dir: &Path) -> BackupStatus {
    status_with(dir, &|| backup_key(dir))
}

/// Deletes every backup archive in `dir/backups`, as the user asks for; returns how many were removed. Only
/// `backup-<unix>.sbk` files are touched, so the live data and anything else in the folder stay. A missing folder
/// holds no backups.
pub fn delete_backups(dir: &Path) -> io::Result<usize> {
    let entries = match std::fs::read_dir(dir.join(BACKUP_DIR)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let Ok(name) = e.file_name().into_string() else { continue };
        let is_backup = name
            .strip_prefix("backup-")
            .and_then(|s| s.strip_suffix(".sbk"))
            .is_some_and(|t| t.parse::<u64>().is_ok());
        if is_backup {
            std::fs::remove_file(e.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Stages `archive` for the next start after checking it is sound.
pub fn stage_with(dir: &Path, archive: &Path, key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<usize> {
    let bytes = std::fs::read(archive)?;
    let n = decode(&open(&bytes, key)?)?.len();
    // The staged copy stays sealed; `apply_pending` opens it with the same keychain key.
    write_atomic(&dir.join(PENDING), &bytes)?;
    Ok(n)
}

/// Stages `archive` for the next start, with the keychain's archive key for a version 2 archive.
pub fn stage(dir: &Path, archive: &Path) -> io::Result<usize> {
    stage_with(dir, archive, &|| backup_key(dir))
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

    #[test]
    fn a_chosen_disk_holds_the_archives_and_the_data_stays_put() {
        assert_eq!(destination_root(Path::new("/home/u/.local/share/bifrost"), ""), PathBuf::from("/home/u/.local/share/bifrost"));
        assert_eq!(destination_root(Path::new("/data"), "/run/media/u/USB"), PathBuf::from("/run/media/u/USB"));
        assert_eq!(backup_path(Path::new("/run/media/u/USB"), 7), PathBuf::from("/run/media/u/USB/backups/backup-7.sbk"));
    }

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
        // an app's own data folder is accepted by a migration, but only inside that folder
        assert!(decode(&encode(&[("appdata/is.x/db".into(), vec![1])])).is_ok());
        assert!(decode(&encode(&[("appdata/../x".into(), vec![1])])).is_err());
    }

    #[test]
    fn write_entries_writes_only_what_the_picker_takes() {
        let dir = scratch("picker");
        let entries = vec![("Notes/a.md".to_string(), b"a".to_vec()), ("Contacts/b".to_string(), b"b".to_vec())];
        let n = write_entries(&dir, &entries, &|name| name.starts_with("Notes/")).unwrap();
        assert_eq!(n, 1);
        assert!(dir.join("Notes/a.md").is_file());
        assert!(!dir.join("Contacts/b").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
        // The archive key lives in the source data directory's keychain, so the restore is given that key.
        assert_eq!(restore_with(&dst, &out, &|| backup_key(&src)).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(dst.join("Notes/1.md")).unwrap(), "note");
        assert!(!dst.join("unrelated.bin").exists());
    }

    #[test]
    fn excluded_items_and_the_folders_above_them_stay_out_of_the_archive() {
        let src = scratch("excl");
        std::fs::create_dir_all(src.join("Notes/private")).unwrap();
        std::fs::write(src.join("Notes/1.md"), "keep").unwrap();
        std::fs::write(src.join("Notes/private/2.md"), "secret").unwrap();
        std::fs::write(src.join("settings.ini"), "a=b\n").unwrap();
        let mut set = bifrost_core::backup_exclusion::ExclusionSet::new();
        set.set_excluded(&src.join("Notes/private").to_string_lossy(), true).unwrap();
        save_exclusions(&src, &set).unwrap();
        assert_eq!(exclusions(&src), set);
        let out = src.join("out.sbk");
        assert_eq!(create(&src, &out).unwrap(), 2);
        let dst = scratch("excl-dst");
        assert_eq!(restore_with(&dst, &out, &|| backup_key(&src)).unwrap(), 2);
        assert!(dst.join("Notes/1.md").exists());
        assert!(!dst.join("Notes/private/2.md").exists());
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

    #[test]
    fn status_counts_archives_and_checks_the_newest() {
        let d = scratch("status");
        std::fs::create_dir_all(d.join(BACKUP_DIR)).unwrap();
        assert_eq!(status_with(&d, &key_a), BackupStatus { count: 0, latest_unix: None, latest_bytes: 0, verified: None });
        std::fs::write(d.join("settings.ini"), "v=1\n").unwrap();
        create_with(&d, &backup_path(&d, 10), &key_a).unwrap();
        create_with(&d, &backup_path(&d, 20), &key_a).unwrap();
        let s = status_with(&d, &key_a);
        assert_eq!(s.count, 2);
        assert_eq!(s.latest_unix, Some(20));
        assert!(s.latest_bytes > 0);
        assert_eq!(s.verified, Some(true));
        // The wrong key does not verify the newest archive, and a tampered archive does not either.
        assert_eq!(status_with(&d, &no_key).verified, Some(false));
        let newest = backup_path(&d, 20);
        let mut bytes = std::fs::read(&newest).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&newest, bytes).unwrap();
        assert_eq!(status_with(&d, &key_a).verified, Some(false));
    }

    #[test]
    fn delete_removes_only_backup_archives() {
        let d = scratch("delete");
        std::fs::create_dir_all(d.join(BACKUP_DIR)).unwrap();
        for t in [5, 100] {
            std::fs::write(backup_path(&d, t), b"x").unwrap();
        }
        std::fs::write(d.join(BACKUP_DIR).join("notes.txt"), b"keep").unwrap();
        std::fs::write(d.join("settings.ini"), b"keep").unwrap();
        assert_eq!(delete_backups(&d).unwrap(), 2);
        assert!(latest(&d).is_none());
        assert!(d.join(BACKUP_DIR).join("notes.txt").exists());
        assert!(d.join("settings.ini").exists());
        assert_eq!(delete_backups(&d).unwrap(), 0);
        assert_eq!(delete_backups(&scratch("delete-none")).unwrap(), 0);
    }

    const KEY: [u8; 32] = [7; 32];
    const OTHER_KEY: [u8; 32] = [9; 32];

    fn key_a() -> io::Result<[u8; 32]> {
        Ok(KEY)
    }

    fn no_key() -> io::Result<[u8; 32]> {
        Err(bad("no key was asked for"))
    }

    #[test]
    fn encrypted_round_trip_with_a_key() {
        let src = scratch("enc-src");
        std::fs::create_dir_all(src.join("Notes")).unwrap();
        std::fs::write(src.join("Notes/1.md"), "secret note").unwrap();
        let out = src.join("b.sbk");
        assert_eq!(create_with(&src, &out, &key_a).unwrap(), 1);
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(MAGIC_V2));
        assert!(!bytes.windows(b"secret note".len()).any(|w| w == b"secret note"), "plaintext in archive");
        let dst = scratch("enc-dst");
        assert_eq!(restore_with(&dst, &out, &key_a).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(dst.join("Notes/1.md")).unwrap(), "secret note");
    }

    #[test]
    fn a_tampered_encrypted_archive_fails() {
        let body = encode(&[("settings.ini".to_string(), b"a=b\n".to_vec())]);
        let sealed = seal(&KEY, &body).unwrap();
        assert_eq!(open(&sealed, &key_a).unwrap(), body);
        let mut t = sealed.clone();
        let i = MAGIC_V2.len() + NONCE_LEN + 3;
        t[i] ^= 1;
        assert!(open(&t, &key_a).is_err(), "flipped ciphertext opened");
        let mut h = sealed.clone();
        h[MAGIC_V2.len() - 1] ^= 1;
        assert!(open(&h, &key_a).is_err(), "changed header opened");
        assert!(open(&sealed[..sealed.len() - 1], &key_a).is_err(), "truncated tag opened");
    }

    #[test]
    fn a_wrong_key_fails() {
        let body = encode(&[("settings.ini".to_string(), b"a=b\n".to_vec())]);
        let sealed = seal(&KEY, &body).unwrap();
        let other = || -> io::Result<[u8; 32]> { Ok(OTHER_KEY) };
        assert!(open(&sealed, &other).is_err());
    }

    #[test]
    fn an_encrypted_archive_does_not_ask_for_a_key_it_does_not_need() {
        let src = scratch("nokey");
        std::fs::write(src.join("settings.ini"), "v=1\n").unwrap();
        let out = src.join("b.sbk");
        create_with(&src, &out, &key_a).unwrap();
        assert!(restore_with(&src, &out, &no_key).is_err(), "restored without its key");
        let plain = std::fs::read(&out).unwrap();
        assert!(open(&plain, &no_key).is_err());
    }

    #[test]
    fn version_one_archives_still_restore_without_a_key() {
        let src = scratch("legacy");
        let out = src.join("old.sbk");
        std::fs::write(&out, encode(&[("settings.ini".to_string(), b"v=old\n".to_vec())])).unwrap();
        let dst = scratch("legacy-dst");
        assert_eq!(restore_with(&dst, &out, &no_key).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(dst.join("settings.ini")).unwrap(), "v=old\n");
        assert_eq!(stage_with(&dst, &out, &no_key).unwrap(), 1);
    }

    #[test]
    fn staged_encrypted_restore_opens_with_the_key() {
        let src = scratch("stage-enc");
        std::fs::write(src.join("settings.ini"), "v=1\n").unwrap();
        let out = src.join("b.sbk");
        create_with(&src, &out, &key_a).unwrap();
        std::fs::write(src.join("settings.ini"), "v=2\n").unwrap();
        assert_eq!(stage_with(&src, &out, &key_a).unwrap(), 1);
        assert!(src.join(PENDING).exists());
        let applied = restore_with(&src, &src.join(PENDING), &key_a).unwrap();
        assert_eq!(applied, 1);
        assert_eq!(std::fs::read_to_string(src.join("settings.ini")).unwrap(), "v=1\n");
    }
}
