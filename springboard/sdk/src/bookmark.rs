//! Bookmarks (Foundation, "File System"): `URL.bookmarkData(options:includingResourceValuesForKeys:relativeTo:)` and
//! `URL(resolvingBookmarkData:options:relativeTo:bookmarkDataIsStale:)`, so an app can keep a reference to a file
//! (a recent document) that still resolves after the file was renamed or moved.
//!
//! Data format: a binary property list (`binary_plist`) with the path, the device and inode numbers, a cache of
//! name, size and modification date, any resource values asked for, and the security scope flags. It is this
//! system's own layout and is not readable by the framework.
//!
//! Resolution: if the stored path still names the same inode on the same device, that is the answer and the
//! bookmark is fresh. Otherwise the item is looked for by inode, first in the directory it was in, then in each
//! ancestor (at most eight levels, never leaving the volume, at most 20000 entries per level), and the bookmark is
//! reported stale so the caller can store a new one. If nothing has that inode but something exists at the stored
//! path, that is returned, stale. Otherwise resolution fails. A bookmark made `relative_to` a directory stores the
//! path below it and resolves against the directory given at resolution time, so a whole folder can move.
//!
//! Security scope: a bookmark made `with_security_scope` resolves to a [`SecurityScope`] that must be started
//! (`start_accessing`) before `read` or `write` on the [`ResolvedBookmark`] succeed, and `write` is refused when
//! the bookmark was made read-only. Starts nest and each needs a stop. A bookmark without a scope has no gate.
//!
//! Gaps: no network volumes or mounting, no bookmark files, no per-process sandbox extension behind the scope (it
//! is enforced by this API only).

use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::binary_plist::{self, BValue};
use crate::file_manager::{NativeFileManager, ResourceKey, ResourceValues};

const FORMAT: &str = "bifrost-bookmark";
const VERSION: i64 = 1;
/// Ancestors searched for a moved item.
const MAX_ANCESTORS: usize = 8;
/// Directory entries examined per ancestor.
const SEARCH_BUDGET: usize = 20_000;

/// `URL.BookmarkCreationOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookmarkCreationOptions {
    /// Store only what resolution needs: no cached name, size or date.
    pub minimal_bookmark: bool,
    pub suitable_for_bookmark_file: bool,
    pub with_security_scope: bool,
    pub security_scope_allows_only_read_access: bool,
}

/// `URL.BookmarkResolutionOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookmarkResolutionOptions {
    pub without_ui: bool,
    pub without_mounting: bool,
    pub with_security_scope: bool,
}

#[derive(Debug)]
pub enum BookmarkError {
    /// The data is not a bookmark of this format.
    Malformed(String),
    /// The item is gone: nothing at the stored path and no item with the stored inode nearby.
    NotFound(PathBuf),
    /// The bookmark was made relative to a directory and none was given.
    MissingBase,
    Io(io::Error),
}

impl fmt::Display for BookmarkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookmarkError::Malformed(m) => write!(f, "malformed bookmark: {m}"),
            BookmarkError::NotFound(p) => write!(f, "the item {} no longer exists", p.display()),
            BookmarkError::MissingBase => write!(f, "the bookmark is relative and no base directory was given"),
            BookmarkError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BookmarkError {}

impl From<io::Error> for BookmarkError {
    fn from(e: io::Error) -> Self {
        BookmarkError::Io(e)
    }
}

/// The access gate of a security-scoped bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecurityScope {
    read_only: bool,
    depth: usize,
}

impl SecurityScope {
    /// `startAccessingSecurityScopedResource()`: always true for a scope.
    pub fn start_accessing(&mut self) -> bool {
        self.depth += 1;
        true
    }

    /// `stopAccessingSecurityScopedResource()`: balances one start. An extra stop does nothing.
    pub fn stop_accessing(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn is_accessing(&self) -> bool {
        self.depth > 0
    }

    pub fn allows_only_read_access(&self) -> bool {
        self.read_only
    }
}

/// What resolving a bookmark gives back.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBookmark {
    pub path: PathBuf,
    /// `bookmarkDataIsStale`: the item moved or was replaced; make a new bookmark and store that.
    pub is_stale: bool,
    pub scope: Option<SecurityScope>,
}

impl ResolvedBookmark {
    fn check(&self, write: bool) -> io::Result<()> {
        if let Some(scope) = &self.scope {
            if !scope.is_accessing() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the security scope is not being accessed"));
            }
            if write && scope.allows_only_read_access() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the security scope is read-only"));
            }
        }
        Ok(())
    }

    /// Reads the item, subject to the scope.
    pub fn read(&self) -> io::Result<Vec<u8>> {
        self.check(false)?;
        fs::read(&self.path)
    }

    /// Writes the item, subject to the scope.
    pub fn write(&self, contents: &[u8]) -> io::Result<()> {
        self.check(true)?;
        fs::write(&self.path, contents)
    }
}

/// `bookmarkData(options:includingResourceValuesForKeys:relativeTo:)`.
pub fn bookmark_data(
    path: &Path,
    options: BookmarkCreationOptions,
    keys: &[ResourceKey],
    relative_to: Option<&Path>,
) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let relative = relative_to.and_then(|base| absolute.strip_prefix(base).ok()).map(|r| r.to_string_lossy().into_owned());
    let mut entries = vec![
        ("format".to_string(), BValue::Str(FORMAT.to_string())),
        ("version".to_string(), BValue::Int(VERSION)),
        ("path".to_string(), BValue::Str(absolute.to_string_lossy().into_owned())),
        ("dev".to_string(), BValue::Int(meta.dev() as i64)),
        ("ino".to_string(), BValue::Int(meta.ino() as i64)),
        ("scope".to_string(), BValue::Bool(options.with_security_scope)),
        ("readOnly".to_string(), BValue::Bool(options.security_scope_allows_only_read_access)),
    ];
    if let Some(r) = relative {
        entries.push(("relative".to_string(), BValue::Str(r)));
    }
    if !options.minimal_bookmark {
        let cached = [ResourceKey::Name, ResourceKey::IsDirectory, ResourceKey::FileSize, ResourceKey::ContentModificationDate];
        let all: Vec<ResourceKey> = cached.iter().chain(keys.iter()).copied().collect();
        let values = NativeFileManager.resource_values(path, &all)?;
        entries.push(("values".to_string(), BValue::Dict(values_to_entries(&values))));
    }
    Ok(binary_plist::write(&BValue::Dict(entries)))
}

/// `URL(resolvingBookmarkData:options:relativeTo:bookmarkDataIsStale:)`.
pub fn resolve_bookmark(
    data: &[u8],
    options: BookmarkResolutionOptions,
    relative_to: Option<&Path>,
) -> Result<ResolvedBookmark, BookmarkError> {
    let root = parse(data)?;
    let stored_path = text(&root, "path")?;
    let dev = number(&root, "dev")? as u64;
    let ino = number(&root, "ino")? as u64;
    let stored = match root.get("relative") {
        Some(BValue::Str(rel)) => match relative_to {
            Some(base) => base.join(rel),
            None => return Err(BookmarkError::MissingBase),
        },
        _ => PathBuf::from(stored_path),
    };
    let (path, is_stale) = if identity(&stored) == Some((dev, ino)) {
        (stored, false)
    } else if let Some(found) = find_moved(&stored, dev, ino) {
        (found, true)
    } else if fs::symlink_metadata(&stored).is_ok() {
        (stored, true)
    } else {
        return Err(BookmarkError::NotFound(stored));
    };
    let scoped = matches!(root.get("scope"), Some(BValue::Bool(true)));
    let scope = if options.with_security_scope && scoped {
        Some(SecurityScope { read_only: matches!(root.get("readOnly"), Some(BValue::Bool(true))), depth: 0 })
    } else {
        None
    };
    Ok(ResolvedBookmark { path, is_stale, scope })
}

/// `URL.resourceValues(forKeys:fromBookmarkData:)`: the values cached in the bookmark, without touching the file.
/// A key the bookmark did not cache comes back `None`.
pub fn resource_values_from_bookmark(data: &[u8], keys: &[ResourceKey]) -> Result<ResourceValues, BookmarkError> {
    let root = parse(data)?;
    let Some(BValue::Dict(cached)) = root.get("values") else {
        return Ok(ResourceValues::default());
    };
    let all = entries_to_values(cached);
    let mut out = ResourceValues::default();
    for key in keys {
        match key {
            ResourceKey::Name => out.name = all.name.clone(),
            ResourceKey::IsDirectory => out.is_directory = all.is_directory,
            ResourceKey::IsRegularFile => out.is_regular_file = all.is_regular_file,
            ResourceKey::IsSymbolicLink => out.is_symbolic_link = all.is_symbolic_link,
            ResourceKey::IsHidden => out.is_hidden = all.is_hidden,
            ResourceKey::FileSize => out.file_size = all.file_size,
            ResourceKey::ContentModificationDate => out.content_modification_date = all.content_modification_date,
            ResourceKey::FileResourceIdentifier => out.file_resource_identifier = all.file_resource_identifier,
            ResourceKey::IsExcludedFromBackup => out.is_excluded_from_backup = all.is_excluded_from_backup,
            ResourceKey::VolumeTotalCapacity
            | ResourceKey::VolumeAvailableCapacity
            | ResourceKey::VolumeAvailableCapacityForImportantUsage => {}
        }
    }
    Ok(out)
}

fn parse(data: &[u8]) -> Result<BValue, BookmarkError> {
    let root = binary_plist::read(data).map_err(|e| BookmarkError::Malformed(e.0))?;
    if root.get("format") != Some(&BValue::Str(FORMAT.to_string())) {
        return Err(BookmarkError::Malformed("not a bookmark".into()));
    }
    match root.get("version") {
        Some(BValue::Int(VERSION)) => Ok(root),
        _ => Err(BookmarkError::Malformed("unsupported version".into())),
    }
}

fn text<'a>(root: &'a BValue, key: &str) -> Result<&'a str, BookmarkError> {
    match root.get(key) {
        Some(BValue::Str(s)) => Ok(s),
        _ => Err(BookmarkError::Malformed(format!("{key} is missing"))),
    }
}

fn number(root: &BValue, key: &str) -> Result<i64, BookmarkError> {
    match root.get(key) {
        Some(BValue::Int(i)) => Ok(*i),
        _ => Err(BookmarkError::Malformed(format!("{key} is missing"))),
    }
}

fn values_to_entries(v: &ResourceValues) -> Vec<(String, BValue)> {
    let mut out = Vec::new();
    let mut put = |k: &str, value: BValue| out.push((k.to_string(), value));
    if let Some(x) = &v.name {
        put("name", BValue::Str(x.clone()));
    }
    if let Some(x) = v.is_directory {
        put("isDirectory", BValue::Bool(x));
    }
    if let Some(x) = v.is_regular_file {
        put("isRegularFile", BValue::Bool(x));
    }
    if let Some(x) = v.is_symbolic_link {
        put("isSymbolicLink", BValue::Bool(x));
    }
    if let Some(x) = v.is_hidden {
        put("isHidden", BValue::Bool(x));
    }
    if let Some(x) = v.file_size {
        put("fileSize", BValue::Int(x as i64));
    }
    if let Some(x) = v.content_modification_date {
        put("modified", BValue::Real(x));
    }
    if let Some((d, i)) = v.file_resource_identifier {
        put("identifier", BValue::Array(vec![BValue::Int(d as i64), BValue::Int(i as i64)]));
    }
    if let Some(x) = v.is_excluded_from_backup {
        put("excludedFromBackup", BValue::Bool(x));
    }
    out
}

fn entries_to_values(entries: &[(String, BValue)]) -> ResourceValues {
    let mut v = ResourceValues::default();
    for (key, value) in entries {
        match (key.as_str(), value) {
            ("name", BValue::Str(s)) => v.name = Some(s.clone()),
            ("isDirectory", BValue::Bool(b)) => v.is_directory = Some(*b),
            ("isRegularFile", BValue::Bool(b)) => v.is_regular_file = Some(*b),
            ("isSymbolicLink", BValue::Bool(b)) => v.is_symbolic_link = Some(*b),
            ("isHidden", BValue::Bool(b)) => v.is_hidden = Some(*b),
            ("fileSize", BValue::Int(i)) => v.file_size = Some(*i as u64),
            ("modified", BValue::Real(r)) => v.content_modification_date = Some(*r),
            ("identifier", BValue::Array(a)) => {
                if let [BValue::Int(d), BValue::Int(i)] = a.as_slice() {
                    v.file_resource_identifier = Some((*d as u64, *i as u64));
                }
            }
            ("excludedFromBackup", BValue::Bool(b)) => v.is_excluded_from_backup = Some(*b),
            _ => {}
        }
    }
    v
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Looks for the item with this inode below the directory it used to be in, then below each ancestor.
fn find_moved(stored: &Path, dev: u64, ino: u64) -> Option<PathBuf> {
    let mut dir = stored.parent();
    for _ in 0..MAX_ANCESTORS {
        let d = dir?;
        if let Ok(meta) = fs::symlink_metadata(d) {
            if meta.dev() != dev {
                return None;
            }
            if meta.is_dir() {
                if let Some(found) = search_tree(d, dev, ino) {
                    return Some(found);
                }
            }
        }
        dir = d.parent();
    }
    None
}

/// Breadth first, so the shallowest match wins. Symbolic links and other volumes are not entered.
fn search_tree(root: &Path, dev: u64, ino: u64) -> Option<PathBuf> {
    let mut budget = SEARCH_BUDGET;
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            let Ok(meta) = entry.metadata() else { continue };
            if meta.dev() != dev {
                continue;
            }
            if meta.ino() == ino {
                return Some(entry.path());
            }
            if meta.is_dir() {
                queue.push_back(entry.path());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> BookmarkCreationOptions {
        BookmarkCreationOptions::default()
    }

    fn resolve(data: &[u8], base: Option<&Path>) -> Result<ResolvedBookmark, BookmarkError> {
        resolve_bookmark(data, BookmarkResolutionOptions::default(), base)
    }

    #[test]
    fn resolves_unmoved_item_as_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), &[], None).unwrap();
        let r = resolve(&data, None).unwrap();
        assert_eq!((r.path, r.is_stale), (f, false));
    }

    #[test]
    fn follows_rename_in_same_dir_and_reports_stale() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), &[], None).unwrap();
        let g = dir.path().join("b.txt");
        fs::rename(&f, &g).unwrap();
        let r = resolve(&data, None).unwrap();
        assert_eq!((r.path, r.is_stale), (g, true));
    }

    #[test]
    fn deleted_item_fails_and_garbage_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), &[], None).unwrap();
        fs::remove_file(&f).unwrap();
        assert!(matches!(resolve(&data, None), Err(BookmarkError::NotFound(_))));
        assert!(matches!(resolve(b"nope", None), Err(BookmarkError::Malformed(_))));
    }

    #[test]
    fn relative_bookmark_survives_moving_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        fs::create_dir(&base).unwrap();
        let f = base.join("a.txt");
        fs::write(&f, "x").unwrap();
        let data = bookmark_data(&f, opts(), &[], Some(&base)).unwrap();
        let other = dir.path().join("other");
        fs::rename(&base, &other).unwrap();
        let r = resolve(&data, Some(&other)).unwrap();
        assert_eq!(r.path, other.join("a.txt"));
        assert!(!r.is_stale);
        assert!(matches!(resolve(&data, None), Err(BookmarkError::MissingBase)));
    }

    #[test]
    fn a_security_scope_gates_reads_until_started() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        let o = BookmarkCreationOptions { with_security_scope: true, ..opts() };
        let data = bookmark_data(&f, o, &[], None).unwrap();
        let mut r = resolve_bookmark(&data, BookmarkResolutionOptions { with_security_scope: true, ..Default::default() }, None).unwrap();
        assert!(r.read().is_err(), "not started");
        r.scope.as_mut().unwrap().start_accessing();
        assert_eq!(r.read().unwrap(), b"x");
    }
}
