//! Synced save directories (`GameSave` parity): an app's save files in one container directory, with the state and
//! conflict model of `bifrost_ui::save_sync`.
//!
//! `SyncedDirectory::open(container_id, data_root)` creates `<data_root>/saves/<container_id>` and returns it. This
//! platform has no cloud sync engine yet, so the directory is always local: `state` is `Local` (or `Closed` after
//! `close`), `finish_syncing` returns at once, and `trigger_pending_upload` finds nothing pending. Files are written
//! atomically (temp file, fsync, rename, fsync of the folder), as `defaults.rs` does. The fingerprint is FNV-1a over
//! the sorted file names and bytes; it detects changes and is not a security hash.
//!
//! Gaps: no cloud container or sync engine, so `Ready`, `Offline`, `Syncing` and `Conflicted` never arise here; no
//! sync progress alert; no version history on disk; `close` does not resume a cloud sync.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub use bifrost_ui::save_sync::{newest, resolve_conflicts, ConflictError, State, Version};

/// The error domain name for save errors (`GameSaveErrorDomain`).
pub const ERROR_DOMAIN: &str = "GameSaveErrorDomain";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A save error. `domain` gives the error domain it belongs to.
#[derive(Debug)]
pub enum SaveError {
    /// The container identifier is empty, too long, or has characters outside `A-Z a-z 0-9 . - _`.
    InvalidContainer(String),
    /// A file name is empty, hidden (starts with a dot), too long, or contains a path separator.
    InvalidName(String),
    /// The directory was closed.
    Closed,
    /// A file operation failed.
    Io(io::Error),
}

impl SaveError {
    pub fn domain(&self) -> &'static str {
        ERROR_DOMAIN
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::InvalidContainer(id) => write!(f, "invalid container identifier {id:?}"),
            SaveError::InvalidName(name) => write!(f, "invalid file name {name:?}"),
            SaveError::Closed => f.write_str("the save directory is closed"),
            SaveError::Io(e) => write!(f, "save file error: {e}"),
        }
    }
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io(e)
    }
}

/// `GameSaveSyncedDirectory`: the save directory of one container.
#[derive(Debug)]
pub struct SyncedDirectory {
    url: PathBuf,
    closed: bool,
}

impl SyncedDirectory {
    /// `openDirectory(containerIdentifier:)`: the directory for `container_id` under `data_root`, created if missing.
    pub fn open(container_id: &str, data_root: &Path) -> Result<Self, SaveError> {
        check_container(container_id)?;
        let url = data_root.join("saves").join(container_id);
        fs::create_dir_all(&url)?;
        Ok(SyncedDirectory { url, closed: false })
    }

    /// `url`: the folder the app reads and writes its save files in.
    pub fn url(&self) -> &Path {
        &self.url
    }

    /// `state`: `Closed` after `close`, otherwise `Local` with the version the files on disk make up.
    pub fn state(&self) -> State {
        if self.closed {
            return State::Closed;
        }
        match self.version() {
            Ok(v) => State::Local(v),
            Err(e) => State::Error(e.to_string()),
        }
    }

    /// `finishSyncing()`: waits for the sync to finish. With no sync engine there is nothing to wait for.
    pub fn finish_syncing(&self) -> Result<(), SaveError> {
        self.check_open()
    }

    /// `triggerPendingUpload()`: starts the upload of pending changes and returns how many it started. Always zero here.
    pub fn trigger_pending_upload(&self) -> Result<usize, SaveError> {
        self.check_open()?;
        Ok(0)
    }

    /// The visible files in the directory, sorted by name. Hidden temporary files are not listed.
    pub fn list_files(&self) -> Result<Vec<String>, SaveError> {
        self.check_open()?;
        Ok(self.names()?)
    }

    /// Reads a file; `None` when it does not exist.
    pub fn read_file(&self, name: &str) -> Result<Option<Vec<u8>>, SaveError> {
        self.check_open()?;
        check_name(name)?;
        match fs::read(self.url.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Replaces a file atomically: the old contents stay until the new ones are synced to disk.
    pub fn write_file(&self, name: &str, data: &[u8]) -> Result<(), SaveError> {
        self.check_open()?;
        check_name(name)?;
        let tmp = self.url.join(format!(".{name}.tmp"));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.url.join(name))?;
        fs::File::open(&self.url)?.sync_all()?;
        Ok(())
    }

    /// Removes a file; `false` when it was not there.
    pub fn remove_file(&self, name: &str) -> Result<bool, SaveError> {
        self.check_open()?;
        check_name(name)?;
        match fs::remove_file(self.url.join(name)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// `close()`: ends use of the directory. Later calls fail with `SaveError::Closed`.
    pub fn close(&mut self) {
        self.closed = true;
    }

    fn check_open(&self) -> Result<(), SaveError> {
        if self.closed {
            Err(SaveError::Closed)
        } else {
            Ok(())
        }
    }

    fn names(&self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.url)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && entry.file_type()?.is_file() {
                out.push(name);
            }
        }
        out.sort();
        Ok(out)
    }

    fn version(&self) -> io::Result<Version> {
        let mut hash = FNV_OFFSET;
        let mut modified = 0u64;
        for name in self.names()? {
            let path = self.url.join(&name);
            hash = fnv(hash, name.as_bytes());
            hash = fnv(hash, &[0]);
            hash = fnv(hash, &fs::read(&path)?);
            let secs = fs::metadata(&path)?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            modified = modified.max(secs);
        }
        Ok(Version { saving_device: device_name(), modified, is_local: true, fingerprint: hash })
    }
}

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        hash = (hash ^ u64::from(*b)).wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The host name, as the device name in a version; a fixed label when the kernel does not show one.
fn device_name() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "This device".to_string())
}

fn check_container(id: &str) -> Result<(), SaveError> {
    let ok = !id.is_empty()
        && id.len() <= 255
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if ok {
        Ok(())
    } else {
        Err(SaveError::InvalidContainer(id.to_string()))
    }
}

fn check_name(name: &str) -> Result<(), SaveError> {
    let ok = !name.is_empty() && name.len() <= 255 && !name.starts_with('.') && !name.contains('/') && !name.contains('\0');
    if ok {
        Ok(())
    } else {
        Err(SaveError::InvalidName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    /// A fresh directory for one test; each test removes its own.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        std::env::temp_dir().join(format!("bifrost-save-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn open_creates_the_container_and_refuses_bad_identifiers() {
        let root = scratch("open");
        let dir = SyncedDirectory::open("is.example.game", &root).expect("valid id opens");
        assert!(dir.url().is_dir());
        assert!(dir.url().ends_with("saves/is.example.game"));
        for bad in ["", ".hidden", "a/b", "a b", "../escape"] {
            assert!(matches!(SyncedDirectory::open(bad, &root), Err(SaveError::InvalidContainer(_))), "{bad:?}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_then_read_round_trips_and_a_missing_file_reads_as_none() {
        let root = scratch("rw");
        let dir = SyncedDirectory::open("is.example.game", &root).unwrap();
        assert_eq!(dir.read_file("slot1.sav").unwrap(), None);
        dir.write_file("slot1.sav", b"level 4").unwrap();
        dir.write_file("slot1.sav", b"level 5").unwrap();
        assert_eq!(dir.read_file("slot1.sav").unwrap().as_deref(), Some(&b"level 5"[..]));
        assert_eq!(dir.list_files().unwrap(), vec!["slot1.sav".to_string()]);
        assert!(dir.remove_file("slot1.sav").unwrap());
        assert!(!dir.remove_file("slot1.sav").unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn temporary_files_stay_hidden_and_bad_names_are_refused() {
        let root = scratch("names");
        let dir = SyncedDirectory::open("is.example.game", &root).unwrap();
        dir.write_file("progress.json", b"{}").unwrap();
        assert!(!dir.list_files().unwrap().iter().any(|n| n.ends_with(".tmp")));
        for bad in ["", ".secret", "../up", "sub/file"] {
            assert!(matches!(dir.write_file(bad, b"x"), Err(SaveError::InvalidName(_))), "{bad:?}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_local_version_fingerprint_follows_the_contents() {
        let root = scratch("fingerprint");
        let dir = SyncedDirectory::open("is.example.game", &root).unwrap();
        dir.write_file("a.sav", b"one").unwrap();
        let first = dir.state();
        let again = dir.state();
        assert_eq!(first, again, "unchanged files give the same version");
        dir.write_file("a.sav", b"two").unwrap();
        let changed = dir.state();
        assert_ne!(first.version().unwrap().fingerprint, changed.version().unwrap().fingerprint);
        assert!(changed.is_usable());
        assert!(changed.version().unwrap().is_local);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_closed_directory_refuses_work_and_reports_closed() {
        let root = scratch("closed");
        let mut dir = SyncedDirectory::open("is.example.game", &root).unwrap();
        dir.write_file("a.sav", b"kept").unwrap();
        dir.close();
        assert_eq!(dir.state(), State::Closed);
        assert!(matches!(dir.write_file("b.sav", b"x"), Err(SaveError::Closed)));
        assert!(matches!(dir.read_file("a.sav"), Err(SaveError::Closed)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn with_no_sync_engine_nothing_is_pending_and_the_error_domain_is_the_save_domain() {
        let root = scratch("pending");
        let dir = SyncedDirectory::open("is.example.game", &root).unwrap();
        assert!(dir.finish_syncing().is_ok());
        assert_eq!(dir.trigger_pending_upload().unwrap(), 0);
        assert_eq!(SaveError::Closed.domain(), "GameSaveErrorDomain");
        assert!(SaveError::InvalidName("x".into()).to_string().contains("invalid file name"));
        let _ = fs::remove_dir_all(&root);
    }
}
