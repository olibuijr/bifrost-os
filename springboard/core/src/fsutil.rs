//! Crash-safe file replacement for the apps' stores: the bytes go to a
//! uniquely named sibling, are synced, renamed over the target, and the
//! directory sync is attempted for rename durability. Its failure is currently
//! best effort, so this helper does not guarantee power-cut durability. A reader sees the
//! old file or the new one, never a zero-length or half-written one.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Replaces `path` with `bytes`, creating its directory when missing.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    // fixed length: the target's own name may already be at the filesystem's limit
    let tmp = dir.join(format!(".write-{}-{}.tmp", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
    replace_from(path, &tmp, bytes.as_ref())
}

fn replace_from(path: &Path, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let written = (|| {
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written.and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Best effort: not every filesystem lets a directory be opened and synced.
    let _ = File::open(dir).and_then(|d| d.sync_all());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-fsutil"))
            .join(format!("{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn replaces_the_file_creates_the_directory_and_leaves_no_temporary() {
        let dir = scratch("replace");
        let path = dir.join("sub/a.txt");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, b"two".as_slice()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let left: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("a.txt")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn exclusive_create_failure_preserves_the_existing_temporary() {
        let dir = scratch("collision");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target");
        let tmp = dir.join("occupied.tmp");
        fs::write(&path, "old").unwrap();
        fs::write(&tmp, "foreign").unwrap();
        assert_eq!(replace_from(&path, &tmp, b"new").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(fs::read_to_string(&tmp).unwrap(), "foreign");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_writers_never_share_a_temporary_name() {
        let dir = scratch("race");
        let path = dir.join("b.txt");
        let writers: Vec<_> = (0..8)
            .map(|i| {
                let p = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        write_atomic(&p, format!("writer {i}")).unwrap();
                    }
                })
            })
            .collect();
        writers.into_iter().for_each(|w| w.join().unwrap());
        assert!(fs::read_to_string(&path).unwrap().starts_with("writer "));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_write_cleans_up_and_keeps_the_old_file() {
        let dir = scratch("fail");
        let path = dir.join("c.txt");
        write_atomic(&path, "keep").unwrap();
        // a non-empty directory in the way makes the rename fail
        let blocked = dir.join("d");
        fs::create_dir_all(blocked.join("x")).unwrap();
        assert!(write_atomic(&blocked, "no").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        let _ = fs::remove_dir_all(dir);
    }
}
