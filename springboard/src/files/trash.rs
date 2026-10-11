//! Recently Deleted: deleting moves an item into a hidden folder beside the
//! storage root, stamped with the deletion time, and anything older than
//! 30 days is purged. Restoring puts it back in a folder of the caller's
//! choosing (numbering the name when taken).

use super::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Days an item stays recoverable.
pub const KEEP_DAYS: u64 = 30;
const DAY: u64 = 86_400;
/// The hidden folder name under a storage root.
pub const DIR: &str = ".RecentlyDeleted";
/// Separates the deletion timestamp from the original name.
const SEP: &str = "__";

/// The trash folder for a storage root.
pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

/// The root a path belongs to: the one it lies under, else the first.
pub fn root_for<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.as_os_str().len()).or(roots.first())
}

/// The stored name of `name` deleted at `now` (unix seconds).
pub fn stamp(name: &str, now: u64) -> String {
    format!("{now}{SEP}{name}")
}

/// Splits a stored name into (deleted-at, original name).
pub fn parse(stored: &str) -> Option<(u64, &str)> {
    let (t, name) = stored.split_once(SEP)?;
    Some((t.parse().ok()?, name))
}

/// Whether an item deleted at `at` is past its 30 days.
pub fn expired(at: u64, now: u64) -> bool {
    now.saturating_sub(at) >= KEEP_DAYS * DAY
}

/// Whole days left before an item deleted at `at` is purged (rounded up).
pub fn days_left(at: u64, now: u64) -> u64 {
    let left = (KEEP_DAYS * DAY).saturating_sub(now.saturating_sub(at));
    left.div_ceil(DAY)
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Moves `path` into `trash` stamped with `now`; returns where it went.
pub fn discard(path: &Path, trash: &Path, now: u64) -> io::Result<PathBuf> {
    std::fs::create_dir_all(trash)?;
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid name"))?.to_string_lossy().into_owned();
    let dst = fs::unique_name(trash, &stamp(&name, now));
    match std::fs::rename(path, &dst) {
        Ok(()) => Ok(dst),
        Err(e) if e.raw_os_error() == Some(18) => {
            // another file system: stage through the move helper, then rename
            let moved = fs::move_to(path, trash)?;
            std::fs::rename(&moved, &dst)?;
            Ok(dst)
        }
        Err(e) => Err(e),
    }
}

/// The Delete action: into Recently Deleted (purging expired items first),
/// or for good when `path` is already in there.
pub fn delete_item(path: &Path, bin: &Path) -> io::Result<()> {
    purge(bin, now_secs());
    if path.starts_with(bin) {
        fs::delete(path)
    } else {
        discard(path, bin, now_secs()).map(|_| ())
    }
}

/// Deletes for good the items in `trash` older than 30 days; returns how many.
pub fn purge(trash: &Path, now: u64) -> usize {
    let Ok(rd) = std::fs::read_dir(trash) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let stored = e.file_name().to_string_lossy().into_owned();
        if parse(&stored).is_some_and(|(at, _)| expired(at, now)) && fs::delete(&e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// Puts a trashed item back into `dir` under its original name.
pub fn restore(item: &Path, dir: &Path) -> io::Result<PathBuf> {
    let stored = item.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = parse(&stored).map(|(_, n)| n.to_string()).unwrap_or(stored);
    let dst = fs::unique_name(dir, &name);
    let moved = fs::move_to(item, dir)?;
    if moved != dst {
        std::fs::rename(&moved, &dst)?;
    }
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-trash-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn stamp_round_trips() {
        assert_eq!(parse(&stamp("a__b.txt", 42)), Some((42, "a__b.txt")));
        assert_eq!(parse("plain"), None);
    }

    #[test]
    fn expiry_after_thirty_days() {
        assert!(!expired(0, 30 * DAY - 1));
        assert!(expired(0, 30 * DAY));
        assert_eq!(days_left(0, 0), 30);
        assert_eq!(days_left(0, DAY + 1), 29);
        assert_eq!(days_left(0, 40 * DAY), 0);
    }

    #[test]
    fn discard_purge_restore() {
        let d = scratch("flow");
        let f = d.join("note.txt");
        std::fs::write(&f, "x").unwrap();
        let t = dir(&d);
        let kept = discard(&f, &t, 1000).unwrap();
        assert!(!f.exists() && kept.exists());
        assert_eq!(purge(&t, 1000 + DAY), 0);
        let back = restore(&kept, &d).unwrap();
        assert_eq!(back, f);
        let again = discard(&back, &t, 1000).unwrap();
        assert_eq!(purge(&t, 1000 + 31 * DAY), 1);
        assert!(!again.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn picks_longest_root() {
        let roots = vec![PathBuf::from("/"), PathBuf::from("/home/u")];
        assert_eq!(root_for(&roots, Path::new("/home/u/a")), Some(&roots[1]));
    }
}
