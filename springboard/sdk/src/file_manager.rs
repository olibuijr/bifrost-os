//! `NativeFileManager` parity (Foundation, "File System"): the everyday item operations an app needs on top of
//! `std::fs`. Existence and attributes, shallow and deep directory listings, directory creation, copy, move and
//! remove for files, symbolic links and whole directory trees. Also the document-saving pieces: an atomic `Data`
//! write, `replaceItemAt` with an optional backup, the standard user directories and option-driven enumeration.
//!
//! Behaviour follows the official file manager semantics where it matters for data safety:
//! - `copy_item` and `move_item` refuse an existing destination with `ErrorKind::AlreadyExists`; nothing is replaced.
//! - Symbolic links are never followed by a copy, a move or a remove. A copied link stays a link, and removing a
//!   link removes only the link, never its target.
//! - `move_item` renames when it can. Across file systems it copies, then removes the source; a failed copy removes
//!   its partial destination and leaves the source in place.
//! - `write_atomically` replaces the file in one rename, so a reader sees the old bytes or the new ones. The parent
//!   directory must exist (`Data.write` does not create it).
//! - `replace_item_at` renames the old item aside first and puts it back if the new item cannot be moved in.
//! - Enumeration lists paths depth first, names sorted within each level, and does not descend into symbolic links,
//!   so a link loop cannot run forever.
//!
//! Also here: `replace_item_at` (atomic replace that keeps the original's metadata and can leave a backup),
//! `enumerator` (a directory enumerator that prefetches resource keys) and `resource_values` (the resource keys
//! `URLResourceValues` carries), plus the user directories (`url_for`, `urls_for`). `FileWrapper` is in
//! `file_wrapper`, bookmarks (with security scopes) in `bookmark`.
//!
//! Gaps: no attributes beyond the resource keys below, no creation of symbolic links, no copy progress or
//! delegate callbacks, and no cloud-synced (ubiquitous) items. The user directories are the XDG-style Linux
//! locations (`$HOME/Documents`, `$XDG_DATA_HOME`, `$XDG_CACHE_HOME`), not the `~/Library` folders. The enumerator
//! lists lazily and skips a subdirectory it cannot read; only an unreadable root is an error.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use crate::file_metadata;

/// The C `struct statvfs` on 64-bit Linux; the kernel fills it, so not every field is read.
#[repr(C)]
#[allow(dead_code)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    spare: [c_int; 6],
}

extern "C" {
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
}

static STAGE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// `EXDEV`: the rename crossed a file system boundary, so `move_item` copies and removes instead.
const EXDEV: i32 = 18;

/// The directory `url(for: .itemReplacementDirectory, ...)` creates beside the item it is asked to be appropriate for.
const ITEM_REPLACEMENT_DIR: &str = ".item-replacement";

/// What an item is. Symbolic links are reported as links, not as the item they point to. Other special files
/// (sockets, pipes, devices) count as `File`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Directory,
    Symlink,
}

/// `attributesOfItem(atPath:)`: the fields the Files app and document saving need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemAttributes {
    pub kind: ItemKind,
    /// Bytes for a file, the length of the target path for a link, and the directory entry's size otherwise.
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// `FileManager.SearchPathDirectory`: the user directories an app asks for. Only the user domain exists here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPathDirectory {
    Desktop,
    Downloads,
    Documents,
    /// `$XDG_DATA_HOME`, or `$HOME/.local/share` when that is unset or not absolute.
    ApplicationSupport,
    /// `$XDG_CACHE_HOME`, or `$HOME/.cache` when that is unset or not absolute.
    Caches,
    /// A scratch directory on the same file system as the item named by `appropriate_for`, so that a replacement
    /// is a rename.
    ItemReplacement,
}

/// `FileManager.DirectoryEnumerationOptions`. The default lists everything, descending into every subdirectory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectoryEnumerationOptions {
    /// `.skipsHiddenFiles`: names that start with a dot, and everything below a hidden directory.
    pub skips_hidden_files: bool,
    /// `.skipsSubdirectoryDescendants`: the directory's own items only, without entering any subdirectory.
    pub skips_subdirectory_descendants: bool,
}

/// `URLResourceKey`: the properties of an item that can be asked for and prefetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKey {
    Name,
    IsDirectory,
    IsRegularFile,
    IsSymbolicLink,
    IsHidden,
    FileSize,
    ContentModificationDate,
    /// The device and inode numbers, which identify the item on its volume.
    FileResourceIdentifier,
    IsExcludedFromBackup,
    VolumeTotalCapacity,
    VolumeAvailableCapacity,
    /// Free space plus what the system can reclaim. Nothing is reclaimable here, so it equals the free space.
    VolumeAvailableCapacityForImportantUsage,
}

/// `URLResourceValues`: a value is `Some` only for a key that was asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceValues {
    pub name: Option<String>,
    pub is_directory: Option<bool>,
    pub is_regular_file: Option<bool>,
    pub is_symbolic_link: Option<bool>,
    pub is_hidden: Option<bool>,
    pub file_size: Option<u64>,
    /// Seconds since 1970.
    pub content_modification_date: Option<f64>,
    /// (device, inode).
    pub file_resource_identifier: Option<(u64, u64)>,
    pub is_excluded_from_backup: Option<bool>,
    pub volume_total_capacity: Option<u64>,
    pub volume_available_capacity: Option<u64>,
    pub volume_available_capacity_for_important_usage: Option<u64>,
}

/// `FileManager.ItemReplacementOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplaceOptions {
    /// Do not carry the original's permissions and extended attributes over to the replacement.
    pub using_new_metadata_only: bool,
    /// Keep the backup item (needs a backup name) instead of deleting it once the replace succeeded.
    pub without_deleting_backup_item: bool,
}

/// One item the enumerator returned, with the resource values prefetched for the keys it was created with.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumeratedItem {
    pub path: PathBuf,
    pub values: ResourceValues,
}

/// `FileManager.DirectoryEnumerator`: a deep, depth-first walk, names sorted within each level. A symbolic link is
/// returned but never entered, and a directory that cannot be read is skipped.
pub struct DirectoryEnumerator {
    stack: Vec<std::vec::IntoIter<PathBuf>>,
    keys: Vec<ResourceKey>,
    options: DirectoryEnumerationOptions,
    /// A directory just returned; its children are pushed on the next call unless `skip_descendants` clears it.
    descend: Option<PathBuf>,
}

impl DirectoryEnumerator {
    /// `skipDescendants()`: do not enter the directory that was just returned.
    pub fn skip_descendants(&mut self) {
        self.descend = None;
    }

    /// How deep the walk is: 1 for the children of the root.
    pub fn level(&self) -> usize {
        self.stack.len()
    }
}

impl Iterator for DirectoryEnumerator {
    type Item = EnumeratedItem;

    fn next(&mut self) -> Option<EnumeratedItem> {
        if let Some(dir) = self.descend.take() {
            if let Ok(names) = read_names(&dir) {
                let children: Vec<PathBuf> = names.into_iter().map(|n| dir.join(n)).collect();
                self.stack.push(children.into_iter());
            }
        }
        loop {
            let path = match self.stack.last_mut()?.next() {
                Some(p) => p,
                None => {
                    self.stack.pop();
                    continue;
                }
            };
            let hidden = path.file_name().map_or(false, |n| n.as_bytes().first() == Some(&b'.'));
            if self.options.skips_hidden_files && hidden {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if kind_of(&meta) == ItemKind::Directory && !self.options.skips_subdirectory_descendants {
                self.descend = Some(path.clone());
            }
            let values = values_for(&path, &meta, &self.keys);
            return Some(EnumeratedItem { path, values });
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeFileManager;

impl NativeFileManager {
    /// `fileExists(atPath:isDirectory:)`: the kind of the item at `path`, or `None` when nothing is there.
    pub fn item_kind(&self, path: &Path) -> Option<ItemKind> {
        fs::symlink_metadata(path).ok().map(|m| kind_of(&m))
    }

    /// `fileExists(atPath:)`.
    pub fn file_exists(&self, path: &Path) -> bool {
        self.item_kind(path).is_some()
    }

    /// `attributesOfItem(atPath:)`. Does not follow a link.
    pub fn attributes_of_item(&self, path: &Path) -> io::Result<ItemAttributes> {
        let m = fs::symlink_metadata(path)?;
        Ok(ItemAttributes { kind: kind_of(&m), size: m.len(), modified: m.modified().ok() })
    }

    /// `contentsOfDirectory(atPath:)`: the immediate children of `dir`, dot files included, sorted by name.
    pub fn contents_of_directory(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(read_names(dir)?.into_iter().map(|n| dir.join(n)).collect())
    }

    /// `subpathsOfDirectory(atPath:)`: every item below `dir`, as paths relative to it, depth first.
    pub fn subpaths_of_directory(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(self
            .enumerator(dir, &[], DirectoryEnumerationOptions::default())?
            .filter_map(|item| item.path.strip_prefix(dir).ok().map(Path::to_path_buf))
            .collect())
    }

    /// `createDirectory(atPath:withIntermediateDirectories:)`. With `intermediates`, an existing directory is not an
    /// error; without it, a missing parent or an existing item is.
    pub fn create_directory(&self, path: &Path, intermediates: bool) -> io::Result<()> {
        if intermediates {
            fs::create_dir_all(path)
        } else {
            fs::create_dir(path)
        }
    }

    /// `Data.write(to:options: .atomic)`: the bytes replace the file at `path` in one rename, so a reader sees the
    /// old contents or the new ones, never a partial write. The parent directory must already exist.
    pub fn write_atomically(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            if !dir.is_dir() {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} does not exist", dir.display())));
            }
        }
        bifrost_core::fsutil::write_atomic(path, bytes)
    }

    /// `url(for:in:appropriateFor:create:)`. `appropriate_for` is needed by `ItemReplacement` only: the scratch
    /// directory goes beside that item (or inside it, when it is a directory). With `create`, the directory is made.
    pub fn url_for(&self, directory: SearchPathDirectory, appropriate_for: Option<&Path>, create: bool) -> io::Result<PathBuf> {
        let path = match directory {
            SearchPathDirectory::ItemReplacement => item_replacement_directory(appropriate_for)?,
            _ => {
                let home = home_dir()?;
                let data_home = env::var_os("XDG_DATA_HOME");
                let cache_home = env::var_os("XDG_CACHE_HOME");
                user_directory(directory, &home, data_home.as_deref(), cache_home.as_deref())
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a user directory"))?
            }
        };
        if create {
            fs::create_dir_all(&path)?;
        }
        Ok(path)
    }

    /// `urls(for:in:)` for the user domain: the one directory `directory` names. Does not create it.
    pub fn urls_for(&self, directory: SearchPathDirectory) -> io::Result<Vec<PathBuf>> {
        Ok(vec![self.url_for(directory, None, false)?])
    }

    /// `temporaryDirectory`: the system's temporary directory (`$TMPDIR`, else `/tmp`).
    pub fn temporary_directory(&self) -> PathBuf {
        env::temp_dir()
    }

    /// The per-user directories a login creates before any app runs: the application support directory and the
    /// cache directory, which is made private (mode 0700) as the per-user cache helper makes it. Existing
    /// directories are left alone. Returns the two paths in that order. The XDG values are read as `url_for` reads them.
    pub fn ensure_login_directories(&self) -> io::Result<Vec<PathBuf>> {
        bifrost_core::login_directories::ensure_for_env()
    }

    /// `removeItem(at:)`: removes a file, a link (not its target) or a directory with everything in it.
    pub fn remove_item(&self, path: &Path) -> io::Result<()> {
        match kind_of(&fs::symlink_metadata(path)?) {
            ItemKind::Directory => fs::remove_dir_all(path),
            ItemKind::File | ItemKind::Symlink => fs::remove_file(path),
        }
    }

    /// `copyItem(at:to:)`: copies a file, a link (as a link) or a whole directory tree to a new path.
    pub fn copy_item(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let m = fs::symlink_metadata(src)?;
        refuse_existing(dst)?;
        copy_tree(src, dst, &m)
    }

    /// `moveItem(at:to:)`: renames the item, or copies and removes it across file systems.
    pub fn move_item(&self, src: &Path, dst: &Path) -> io::Result<()> {
        refuse_existing(dst)?;
        match fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(EXDEV) => {
                let m = fs::symlink_metadata(src)?;
                // `copy_tree` removes its own partial output when it fails.
                copy_tree(src, dst, &m)?;
                self.remove_item(src)
            }
            Err(e) => Err(e),
        }
    }

    /// `resourceValues(forKeys:)`: the requested properties of the item at `path`. Does not follow a link.
    pub fn resource_values(&self, path: &Path, keys: &[ResourceKey]) -> io::Result<ResourceValues> {
        let meta = fs::symlink_metadata(path)?;
        Ok(values_for(path, &meta, keys))
    }

    /// `enumerator(at:includingPropertiesForKeys:options:)`. The root must be a readable directory.
    pub fn enumerator(&self, dir: &Path, keys: &[ResourceKey], options: DirectoryEnumerationOptions) -> io::Result<DirectoryEnumerator> {
        let root: Vec<PathBuf> = read_names(dir)?.into_iter().map(|n| dir.join(n)).collect();
        Ok(DirectoryEnumerator { stack: vec![root.into_iter()], keys: keys.to_vec(), options, descend: None })
    }

    /// `replaceItemAt(_:withItemAt:backupItemName:options:)`: puts `with_item` at the path of `original` and
    /// returns that path.
    ///
    /// A file replacing a file is one atomic rename, so a reader sees the old or the new item, never a mixture.
    /// Anything else (a directory, or a backup) moves the original aside first and puts it back if the move of
    /// the replacement fails. Unless `using_new_metadata_only`, the replacement takes the original's permissions
    /// and extended attributes. With `backup_item_name`, the original is kept next to itself under that name and
    /// deleted again after success unless `without_deleting_backup_item`; an older item under that name is
    /// replaced. A replacement on another volume is copied next to the original first.
    pub fn replace_item_at(
        &self,
        original: &Path,
        with_item: &Path,
        backup_item_name: Option<&str>,
        options: ReplaceOptions,
    ) -> io::Result<PathBuf> {
        let orig_meta = fs::symlink_metadata(original)?;
        let new_meta = fs::symlink_metadata(with_item)?;
        if let Some(name) = backup_item_name {
            if name.is_empty() || name.contains('/') {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "the backup name must be a plain file name"));
            }
        }
        let staged = if orig_meta.dev() != new_meta.dev() {
            let tmp = sibling_temp(original);
            copy_tree(with_item, &tmp, &new_meta)?;
            Some(tmp)
        } else {
            None
        };
        let source: &Path = staged.as_deref().unwrap_or(with_item);
        if !options.using_new_metadata_only {
            if kind_of(&new_meta) != ItemKind::Symlink {
                let _ = fs::set_permissions(source, fs::Permissions::from_mode(orig_meta.mode() & 0o7777));
            }
            let _ = file_metadata::copy_xattrs(original, source);
        }
        let result = self.swap_in(original, source, &orig_meta, &new_meta, backup_item_name, options);
        match (&result, &staged) {
            (Ok(_), Some(_)) => {
                let _ = self.remove_item(with_item);
            }
            (Err(_), Some(tmp)) => {
                let _ = self.remove_item(tmp);
            }
            _ => {}
        }
        result.map(|()| original.to_path_buf())
    }

    fn swap_in(
        &self,
        original: &Path,
        source: &Path,
        orig_meta: &fs::Metadata,
        new_meta: &fs::Metadata,
        backup_item_name: Option<&str>,
        options: ReplaceOptions,
    ) -> io::Result<()> {
        let backup = backup_item_name.map(|n| original.with_file_name(n));
        let files_only = kind_of(orig_meta) != ItemKind::Directory && kind_of(new_meta) != ItemKind::Directory;
        if backup.is_none() && files_only {
            return fs::rename(source, original);
        }
        let aside = backup.clone().unwrap_or_else(|| sibling_temp(original));
        if backup.is_some() && fs::symlink_metadata(&aside).is_ok() {
            self.remove_item(&aside)?;
        }
        fs::rename(original, &aside)?;
        if let Err(e) = fs::rename(source, original) {
            let _ = fs::rename(&aside, original);
            return Err(e);
        }
        if backup.is_none() || !options.without_deleting_backup_item {
            let _ = self.remove_item(&aside);
        }
        Ok(())
    }
}

/// A fresh hidden name next to `path`, for staging a replacement.
fn sibling_temp(path: &Path) -> PathBuf {
    let n = STAGE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.stage-{}-{n}", std::process::id()))
}

fn values_for(path: &Path, meta: &fs::Metadata, keys: &[ResourceKey]) -> ResourceValues {
    let kind = kind_of(meta);
    let mut v = ResourceValues::default();
    let mut volume: Option<(u64, u64)> = None;
    for key in keys {
        match key {
            ResourceKey::Name => {
                v.name = Some(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            }
            ResourceKey::IsDirectory => v.is_directory = Some(kind == ItemKind::Directory),
            ResourceKey::IsRegularFile => v.is_regular_file = Some(meta.file_type().is_file()),
            ResourceKey::IsSymbolicLink => v.is_symbolic_link = Some(kind == ItemKind::Symlink),
            ResourceKey::IsHidden => {
                v.is_hidden = Some(path.file_name().map_or(false, |n| n.as_bytes().first() == Some(&b'.')))
            }
            ResourceKey::FileSize => v.file_size = Some(meta.len()),
            ResourceKey::ContentModificationDate => {
                v.content_modification_date = Some(meta.mtime() as f64 + meta.mtime_nsec() as f64 / 1e9)
            }
            ResourceKey::FileResourceIdentifier => v.file_resource_identifier = Some((meta.dev(), meta.ino())),
            ResourceKey::IsExcludedFromBackup => {
                v.is_excluded_from_backup = Some(file_metadata::is_excluded_from_backup(path))
            }
            ResourceKey::VolumeTotalCapacity
            | ResourceKey::VolumeAvailableCapacity
            | ResourceKey::VolumeAvailableCapacityForImportantUsage => {
                let (total, free) = *volume.get_or_insert_with(|| volume_capacity(path).unwrap_or((0, 0)));
                match key {
                    ResourceKey::VolumeTotalCapacity => v.volume_total_capacity = Some(total),
                    ResourceKey::VolumeAvailableCapacity => v.volume_available_capacity = Some(free),
                    _ => v.volume_available_capacity_for_important_usage = Some(free),
                }
            }
        }
    }
    v
}

/// (total bytes, bytes available to an unprivileged user) of the volume that holds `path`.
fn volume_capacity(path: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buf = StatVfs {
        f_bsize: 0,
        f_frsize: 0,
        f_blocks: 0,
        f_bfree: 0,
        f_bavail: 0,
        f_files: 0,
        f_ffree: 0,
        f_favail: 0,
        f_fsid: 0,
        f_flag: 0,
        f_namemax: 0,
        spare: [0; 6],
    };
    // SAFETY: `c` is a live NUL-terminated path and `buf` matches the C struct's layout on 64-bit Linux.
    let rc = unsafe { statvfs(c.as_ptr(), &mut buf) };
    if rc != 0 {
        return None;
    }
    let unit = if buf.f_frsize != 0 { buf.f_frsize } else { buf.f_bsize };
    Some((buf.f_blocks.saturating_mul(unit), buf.f_bavail.saturating_mul(unit)))
}

fn kind_of(m: &fs::Metadata) -> ItemKind {
    let t = m.file_type();
    if t.is_symlink() {
        ItemKind::Symlink
    } else if t.is_dir() {
        ItemKind::Directory
    } else {
        ItemKind::File
    }
}

/// The names in `dir`, sorted. A directory that cannot be read is an error, not an empty listing.
fn read_names(dir: &Path) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}

/// The user directory for `directory`, under `home`. `data_home` and `cache_home` are the `XDG_*` values; a relative
/// value is ignored, as the XDG base directory rules say. `ItemReplacement` is not a user directory.
fn user_directory(directory: SearchPathDirectory, home: &Path, data_home: Option<&OsStr>, cache_home: Option<&OsStr>) -> Option<PathBuf> {
    Some(match directory {
        SearchPathDirectory::Desktop => home.join("Desktop"),
        SearchPathDirectory::Downloads => home.join("Downloads"),
        SearchPathDirectory::Documents => home.join("Documents"),
        SearchPathDirectory::ApplicationSupport => base_dir(data_home, home, ".local/share"),
        SearchPathDirectory::Caches => base_dir(cache_home, home, ".cache"),
        SearchPathDirectory::ItemReplacement => return None,
    })
}

/// An XDG base directory: the variable when it holds an absolute path, else `home/fallback`.
fn base_dir(var: Option<&OsStr>, home: &Path, fallback: &str) -> PathBuf {
    match var.map(Path::new) {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        _ => home.join(fallback),
    }
}

fn item_replacement_directory(appropriate_for: Option<&Path>) -> io::Result<PathBuf> {
    let item = appropriate_for.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "the item replacement directory needs an appropriate item")
    })?;
    let dir = if item.is_dir() { item } else { parent_dir(item) };
    Ok(dir.join(ITEM_REPLACEMENT_DIR))
}

fn home_dir() -> io::Result<PathBuf> {
    env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

/// The directory an item lives in: its parent, or the current directory for a bare name.
fn parent_dir(path: &Path) -> &Path {
    path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

fn refuse_existing(dst: &Path) -> io::Result<()> {
    if fs::symlink_metadata(dst).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dst.display())));
    }
    Ok(())
}

/// Copies the item `src` (whose metadata is `m`, not followed) to `dst`, which must not exist. All or nothing: when
/// the copy fails, everything this call created under `dst` is removed again (nothing that was there before is
/// touched, because `dst` itself is only removed once this call has made it) and the original error is returned.
fn copy_tree(src: &Path, dst: &Path, m: &fs::Metadata) -> io::Result<()> {
    match kind_of(m) {
        ItemKind::File => {
            let mut from = fs::File::open(src)?;
            // `create_new` fails on an existing path, so a file made by someone else is never overwritten.
            let mut to = fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
            let copied = io::copy(&mut from, &mut to).and_then(|_| to.set_permissions(m.permissions()));
            if let Err(e) = copied {
                drop(to);
                let _ = fs::remove_file(dst);
                return Err(e);
            }
        }
        ItemKind::Symlink => std::os::unix::fs::symlink(fs::read_link(src)?, dst)?,
        ItemKind::Directory => {
            fs::create_dir(dst)?;
            let filled = (|| -> io::Result<()> {
                for name in read_names(src)? {
                    let from = src.join(&name);
                    let child = fs::symlink_metadata(&from)?;
                    copy_tree(&from, &dst.join(&name), &child)?;
                }
                // Permissions last, so a read-only directory still takes its children.
                fs::set_permissions(dst, m.permissions())
            })();
            if let Err(e) = filled {
                let _ = fs::remove_dir_all(dst);
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh scratch directory under this crate's `target/`, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let p = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-file-manager"))
                .join(format!("{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn touch(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// The names of `paths` relative to `base`, as strings, in the order given.
    fn relative(base: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
        paths.into_iter().map(|p| p.strip_prefix(base).unwrap().display().to_string()).collect()
    }

    #[test]
    fn copy_item_copies_a_whole_directory_tree() {
        let s = Scratch::new();
        touch(&s.0.join("a/x.txt"), "x");
        touch(&s.0.join("a/sub/y.txt"), "yy");
        NativeFileManager.copy_item(&s.0.join("a"), &s.0.join("b")).unwrap();
        assert_eq!(fs::read_to_string(s.0.join("b/x.txt")).unwrap(), "x");
        assert_eq!(fs::read_to_string(s.0.join("b/sub/y.txt")).unwrap(), "yy");
        assert!(s.0.join("a/sub/y.txt").exists(), "the source stays");
    }

    #[test]
    fn a_failed_directory_copy_removes_what_it_created_and_keeps_pre_existing_files() {
        let s = Scratch::new();
        // A tree whose last entry cannot be read, so the copy fails after it has written the entries before it.
        touch(&s.0.join("a/one.txt"), "1");
        touch(&s.0.join("a/sub/two.txt"), "2");
        touch(&s.0.join("a/zz.txt"), "locked");
        fs::set_permissions(s.0.join("a/zz.txt"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(s.0.join("a/zz.txt")).is_ok() {
            return; // running as root: permissions do not stop the read, so there is no failure to simulate
        }
        touch(&s.0.join("keep/existing.txt"), "mine");
        let fm = NativeFileManager;

        let err = fm.copy_item(&s.0.join("a"), &s.0.join("keep/b")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "the original error comes back");
        assert!(fs::symlink_metadata(s.0.join("keep/b")).is_err(), "the partial destination is gone");
        assert_eq!(fs::read_to_string(s.0.join("keep/existing.txt")).unwrap(), "mine", "pre-existing files survive");
        assert!(s.0.join("a/sub/two.txt").exists(), "the source is untouched");

        // A refused copy (destination exists) never removes the existing destination.
        let err = fm.copy_item(&s.0.join("a"), &s.0.join("keep")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(s.0.join("keep/existing.txt")).unwrap(), "mine");
    }

    #[test]
    fn copy_and_move_refuse_an_existing_destination() {
        let s = Scratch::new();
        touch(&s.0.join("src.txt"), "new");
        touch(&s.0.join("dst.txt"), "old");
        let fm = NativeFileManager;
        assert_eq!(fm.copy_item(&s.0.join("src.txt"), &s.0.join("dst.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fm.move_item(&s.0.join("src.txt"), &s.0.join("dst.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(s.0.join("dst.txt")).unwrap(), "old", "nothing was replaced");
        assert!(s.0.join("src.txt").exists(), "nothing was moved");
    }

    #[test]
    fn move_item_renames_and_the_source_is_gone() {
        let s = Scratch::new();
        touch(&s.0.join("note.txt"), "hello");
        fs::create_dir_all(s.0.join("moved")).unwrap();
        NativeFileManager.move_item(&s.0.join("note.txt"), &s.0.join("moved/note.txt")).unwrap();
        assert!(!s.0.join("note.txt").exists());
        assert_eq!(fs::read_to_string(s.0.join("moved/note.txt")).unwrap(), "hello");
    }

    #[test]
    fn subpaths_are_relative_depth_first_and_sorted() {
        let s = Scratch::new();
        touch(&s.0.join("b.txt"), "");
        touch(&s.0.join("a/z.txt"), "");
        touch(&s.0.join("a/m/k.txt"), "");
        let got: Vec<String> = NativeFileManager
            .subpaths_of_directory(&s.0)
            .unwrap()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, vec!["a", "a/m", "a/m/k.txt", "a/z.txt", "b.txt"]);
        let shallow = NativeFileManager.contents_of_directory(&s.0).unwrap();
        assert_eq!(shallow, vec![s.0.join("a"), s.0.join("b.txt")]);
    }

    #[test]
    fn enumerator_skips_hidden_files_and_descendants_per_options() {
        let s = Scratch::new();
        touch(&s.0.join("visible/a.txt"), "");
        touch(&s.0.join("visible/.dot.txt"), "");
        touch(&s.0.join(".hidden/secret.txt"), "");
        touch(&s.0.join(".env"), "");
        let fm = NativeFileManager;
        let list = |o| relative(&s.0, fm.enumerator(&s.0, &[], o).unwrap().map(|i| i.path));

        let all = list(DirectoryEnumerationOptions::default());
        assert_eq!(all, vec![".env", ".hidden", ".hidden/secret.txt", "visible", "visible/.dot.txt", "visible/a.txt"]);

        let visible_only = list(DirectoryEnumerationOptions { skips_hidden_files: true, ..Default::default() });
        assert_eq!(visible_only, vec!["visible", "visible/a.txt"], "hidden names and their descendants are skipped");

        let shallow = list(DirectoryEnumerationOptions { skips_subdirectory_descendants: true, ..Default::default() });
        assert_eq!(shallow, vec![".env", ".hidden", "visible"]);

        let both = list(DirectoryEnumerationOptions { skips_hidden_files: true, skips_subdirectory_descendants: true });
        assert_eq!(both, vec!["visible"]);
    }

    #[test]
    fn enumerator_yields_full_paths_and_fails_on_a_missing_directory() {
        let s = Scratch::new();
        touch(&s.0.join("a.txt"), "");
        let fm = NativeFileManager;
        let first = fm.enumerator(&s.0, &[], DirectoryEnumerationOptions::default()).unwrap().next().unwrap();
        assert_eq!(first.path, s.0.join("a.txt"));
        assert!(fm.enumerator(&s.0.join("missing"), &[], DirectoryEnumerationOptions::default()).is_err());
    }

    #[test]
    fn symbolic_links_are_listed_but_never_entered_or_followed_on_remove() {
        let s = Scratch::new();
        touch(&s.0.join("real/inside.txt"), "data");
        std::os::unix::fs::symlink(s.0.join("real"), s.0.join("real/loop")).unwrap();
        let fm = NativeFileManager;
        let paths: Vec<String> = fm.subpaths_of_directory(&s.0).unwrap().iter().map(|p| p.display().to_string()).collect();
        assert!(paths.contains(&"real/loop".to_string()), "the link is listed");
        assert!(!paths.iter().any(|p| p.starts_with("real/loop/")), "the link is not entered");
        assert_eq!(fm.item_kind(&s.0.join("real/loop")), Some(ItemKind::Symlink));

        fm.remove_item(&s.0.join("real/loop")).unwrap();
        assert!(s.0.join("real/inside.txt").exists(), "removing a link keeps its target");
        assert!(!fm.file_exists(&s.0.join("real/loop")));
    }

    #[test]
    fn attributes_report_kind_size_and_absence() {
        let s = Scratch::new();
        touch(&s.0.join("five.txt"), "12345");
        fs::create_dir(s.0.join("dir")).unwrap();
        let fm = NativeFileManager;
        let a = fm.attributes_of_item(&s.0.join("five.txt")).unwrap();
        assert_eq!((a.kind, a.size), (ItemKind::File, 5));
        assert!(a.modified.is_some());
        assert_eq!(fm.item_kind(&s.0.join("dir")), Some(ItemKind::Directory));
        assert_eq!(fm.item_kind(&s.0.join("missing")), None);
        assert!(fm.attributes_of_item(&s.0.join("missing")).is_err());
    }

    #[test]
    fn create_directory_with_and_without_intermediates() {
        let s = Scratch::new();
        let fm = NativeFileManager;
        assert!(fm.create_directory(&s.0.join("a/b/c"), false).is_err(), "parents are required");
        fm.create_directory(&s.0.join("a/b/c"), true).unwrap();
        fm.create_directory(&s.0.join("a/b/c"), true).unwrap();
        assert_eq!(fm.item_kind(&s.0.join("a/b/c")), Some(ItemKind::Directory));
    }

    #[test]
    fn write_atomically_replaces_content_and_leaves_no_temporary() {
        let s = Scratch::new();
        let path = s.0.join("doc.json");
        let fm = NativeFileManager;
        fm.write_atomically(&path, b"one").unwrap();
        fm.write_atomically(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<OsString> = fs::read_dir(&s.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("doc.json")], "no temporary file is left behind");
    }

    #[test]
    fn write_atomically_refuses_a_missing_parent_directory() {
        let s = Scratch::new();
        let err = NativeFileManager.write_atomically(&s.0.join("missing/doc.json"), b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!s.0.join("missing").exists(), "the directory is not created");
    }

    #[test]
    fn replace_item_at_swaps_content_and_keeps_a_named_backup() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "old");
        touch(&s.0.join("new.txt"), "new");
        let placed = NativeFileManager
            .replace_item_at(&s.0.join("doc.txt"), &s.0.join("new.txt"), Some("doc.bak"), ReplaceOptions { without_deleting_backup_item: true, ..Default::default() })
            .unwrap();
        assert_eq!(placed, s.0.join("doc.txt"));
        assert_eq!(fs::read_to_string(s.0.join("doc.txt")).unwrap(), "new");
        assert_eq!(fs::read_to_string(s.0.join("doc.bak")).unwrap(), "old", "the old item is the backup");
        assert!(!s.0.join("new.txt").exists(), "the new item moved into place");
    }

    #[test]
    fn replace_item_at_without_a_backup_name_leaves_only_the_new_item() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "old");
        touch(&s.0.join("new.txt"), "new");
        NativeFileManager.replace_item_at(&s.0.join("doc.txt"), &s.0.join("new.txt"), None, ReplaceOptions::default()).unwrap();
        assert_eq!(fs::read_to_string(s.0.join("doc.txt")).unwrap(), "new");
        let names: Vec<OsString> = fs::read_dir(&s.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("doc.txt")], "the old item is deleted");
    }

    #[test]
    fn replace_item_at_needs_both_items_and_changes_nothing_when_one_is_missing() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "old");
        let fm = NativeFileManager;
        let err = fm.replace_item_at(&s.0.join("doc.txt"), &s.0.join("absent.txt"), None, ReplaceOptions::default()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(s.0.join("doc.txt")).unwrap(), "old");
    }

    #[test]
    fn user_directories_resolve_under_home_and_honour_xdg_paths() {
        let home = Path::new("/home/ada");
        let at = |d, data: Option<&str>, cache: Option<&str>| {
            user_directory(d, home, data.map(OsStr::new), cache.map(OsStr::new)).unwrap()
        };
        assert_eq!(at(SearchPathDirectory::Documents, None, None), PathBuf::from("/home/ada/Documents"));
        assert_eq!(at(SearchPathDirectory::Desktop, None, None), PathBuf::from("/home/ada/Desktop"));
        assert_eq!(at(SearchPathDirectory::Downloads, None, None), PathBuf::from("/home/ada/Downloads"));
        assert_eq!(at(SearchPathDirectory::ApplicationSupport, None, None), PathBuf::from("/home/ada/.local/share"));
        assert_eq!(at(SearchPathDirectory::Caches, None, None), PathBuf::from("/home/ada/.cache"));
        assert_eq!(at(SearchPathDirectory::ApplicationSupport, Some("/data"), None), PathBuf::from("/data"));
        assert_eq!(at(SearchPathDirectory::Caches, None, Some("/cache")), PathBuf::from("/cache"));
        assert_eq!(at(SearchPathDirectory::Caches, None, Some("relative")), PathBuf::from("/home/ada/.cache"), "relative XDG values are ignored");
        assert_eq!(user_directory(SearchPathDirectory::ItemReplacement, home, None, None), None);
    }

    #[test]
    fn url_for_creates_the_item_replacement_directory_beside_the_item() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "x");
        let fm = NativeFileManager;
        let dir = fm.url_for(SearchPathDirectory::ItemReplacement, Some(&s.0.join("doc.txt")), true).unwrap();
        assert_eq!(dir, s.0.join(ITEM_REPLACEMENT_DIR));
        assert!(dir.is_dir());
        let err = fm.url_for(SearchPathDirectory::ItemReplacement, None, false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "the replacement directory needs an appropriate item");
    }

    #[test]
    fn url_for_does_not_create_the_directory_unless_asked() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "x");
        let dir = NativeFileManager
            .url_for(SearchPathDirectory::ItemReplacement, Some(&s.0.join("doc.txt")), false)
            .unwrap();
        assert_eq!(dir, s.0.join(ITEM_REPLACEMENT_DIR));
        assert!(!dir.exists());
    }
}
