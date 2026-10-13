//! URL bookmarks: `URL.bookmarkData(options:includingResourceValuesForKeys:relativeTo:)` and
//! `URL(resolvingBookmarkData:options:relativeTo:bookmarkDataIsStale:)`.
//!
//! The blob records the path plus the device, inode and size of the item. Resolving first tries
//! the recorded path; if the item moved or was renamed inside its old directory, the directory
//! is scanned for the same device and inode and the bookmark is reported stale so the caller
//! can store a fresh one. Security scopes are not supported and resource values are not stored.

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"bmk0";

/// `URL.BookmarkCreationOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookmarkCreationOptions {
    pub minimal_bookmark: bool,
    pub with_security_scope: bool,
}

/// The result of resolving a bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBookmark {
    pub path: PathBuf,
    /// `bookmarkDataIsStale`: the item was found somewhere other than the recorded path.
    pub is_stale: bool,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Makes bookmark data for an existing item. A `relative_to` base stores the path relative to it.
pub fn bookmark_data(
    path: &Path,
    options: BookmarkCreationOptions,
    relative_to: Option<&Path>,
) -> io::Result<Vec<u8>> {
    if options.with_security_scope {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "security scope"));
    }
    let meta = std::fs::metadata(path)?;
    let stored = match relative_to {
        Some(base) => path.strip_prefix(base).map_err(|_| invalid("not under relativeTo"))?,
        None => path,
    };
    let mut out = MAGIC.to_vec();
    out.push(relative_to.is_some() as u8);
    out.extend(meta.dev().to_le_bytes());
    out.extend(meta.ino().to_le_bytes());
    out.extend(meta.size().to_le_bytes());
    out.extend(stored.as_os_str().as_bytes());
    Ok(out)
}

/// Resolves bookmark data made by [`bookmark_data`].
pub fn resolve_bookmark(data: &[u8], relative_to: Option<&Path>) -> io::Result<ResolvedBookmark> {
    if data.len() < 29 || &data[..4] != MAGIC {
        return Err(invalid("not bookmark data"));
    }
    let word = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
    let (dev, ino) = (word(5), word(13));
    let raw = PathBuf::from(std::ffi::OsStr::from_bytes(&data[29..]));
    let path = if data[4] == 1 {
        relative_to.ok_or_else(|| invalid("relativeTo required"))?.join(raw)
    } else {
        raw
    };
    let same = |p: &Path| std::fs::metadata(p).map(|m| m.dev() == dev && m.ino() == ino).unwrap_or(false);
    if same(&path) {
        return Ok(ResolvedBookmark { path, is_stale: false });
    }
    if let Some(dir) = path.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if same(&entry.path()) {
                    return Ok(ResolvedBookmark { path: entry.path(), is_stale: true });
                }
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "bookmark target is gone"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn opts() -> BookmarkCreationOptions {
        BookmarkCreationOptions::default()
    }

    #[test]
    fn resolves_unmoved_item_as_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), None).unwrap();
        assert_eq!(resolve_bookmark(&data, None).unwrap(), ResolvedBookmark { path: f, is_stale: false });
    }

    #[test]
    fn follows_rename_in_same_dir_and_reports_stale() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), None).unwrap();
        let g = dir.path().join("b.txt");
        fs::rename(&f, &g).unwrap();
        assert_eq!(resolve_bookmark(&data, None).unwrap(), ResolvedBookmark { path: g, is_stale: true });
    }

    #[test]
    fn deleted_item_fails_and_garbage_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), None).unwrap();
        fs::remove_file(&f).unwrap();
        assert_eq!(resolve_bookmark(&data, None).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(resolve_bookmark(b"nope", None).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn relative_bookmark_survives_moving_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        fs::create_dir(&base).unwrap();
        let f = base.join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), Some(&base)).unwrap();
        let other = dir.path().join("other");
        fs::rename(&base, &other).unwrap();
        let r = resolve_bookmark(&data, Some(&other)).unwrap();
        assert_eq!(r.path, other.join("a.txt"));
        assert!(!r.is_stale);
        assert!(resolve_bookmark(&data, None).is_err());
    }

    #[test]
    fn security_scope_is_unsupported() {
        let o = BookmarkCreationOptions { with_security_scope: true, ..opts() };
        let e = bookmark_data(Path::new("/"), o, None).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
    }
}
