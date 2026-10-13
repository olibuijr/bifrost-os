//! `NativeFileManager` parity (Foundation, "File System"): the everyday item operations an app needs on top of
//! `std::fs`. Existence and attributes, shallow and deep directory listings, directory creation, copy, move and
//! remove for files, symbolic links and whole directory trees.
//!
//! Behaviour follows the official file manager semantics where it matters for data safety:
//! - `copy_item` and `move_item` refuse an existing destination with `ErrorKind::AlreadyExists`; nothing is replaced.
//! - Symbolic links are never followed by a copy, a move or a remove. A copied link stays a link, and removing a
//!   link removes only the link, never its target.
//! - `move_item` renames when it can. Across file systems it copies, then removes the source; a failed copy removes
//!   its partial destination and leaves the source in place.
//! - Deep enumeration lists paths relative to the directory, depth first, names sorted within each level, and does
//!   not descend into symbolic links, so a link loop cannot run forever.
//!
//! Also here: `replace_item_at` (atomic replace that keeps the original's metadata and can leave a backup),
//! `enumerator` (a directory enumerator that prefetches resource keys) and `resource_values` (the resource keys
//! `URLResourceValues` carries). `FileWrapper` is in `file_wrapper`, bookmarks in `bookmark`.
//!
//! Gaps: no attributes beyond the resource keys below, no creation of symbolic links, no copy progress or
//! delegate callbacks, and no cloud-synced (ubiquitous) items.

use std::ffi::OsString;
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

/// `FileManager.DirectoryEnumerationOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnumerationOptions {
    pub skips_subdirectory_descendants: bool,
    pub skips_hidden_files: bool,
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
    options: EnumerationOptions,
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
        let mut out = Vec::new();
        walk(dir, Path::new(""), &mut out)?;
        Ok(out)
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
                if let Err(e) = copy_tree(src, dst, &m) {
                    let _ = self.remove_item(dst);
                    return Err(e);
                }
                self.remove_item(src)
            }
            Err(e) => Err(e),
        }
    }

    /// `removeItem(at:)`: removes a file, a link (not its target) or a directory with everything in it.
    pub fn remove_item(&self, path: &Path) -> io::Result<()> {
        match kind_of(&fs::symlink_metadata(path)?) {
            ItemKind::Directory => fs::remove_dir_all(path),
            ItemKind::File | ItemKind::Symlink => fs::remove_file(path),
        }
    }

    /// `resourceValues(forKeys:)`: the requested properties of the item at `path`. Does not follow a link.
    pub fn resource_values(&self, path: &Path, keys: &[ResourceKey]) -> io::Result<ResourceValues> {
        let meta = fs::symlink_metadata(path)?;
        Ok(values_for(path, &meta, keys))
    }

    /// `enumerator(at:includingPropertiesForKeys:options:)`. The root must be a readable directory.
    pub fn enumerator(&self, dir: &Path, keys: &[ResourceKey], options: EnumerationOptions) -> io::Result<DirectoryEnumerator> {
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

/// Pre-order walk. A symbolic link is listed but never entered.
fn walk(base: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for name in read_names(&base.join(rel))? {
        let child = rel.join(&name);
        let is_real_dir = fs::symlink_metadata(base.join(&child)).map(|m| kind_of(&m) == ItemKind::Directory)?;
        out.push(child.clone());
        if is_real_dir {
            walk(base, &child, out)?;
        }
    }
    Ok(())
}

fn refuse_existing(dst: &Path) -> io::Result<()> {
    if fs::symlink_metadata(dst).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dst.display())));
    }
    Ok(())
}

/// Copies the item `src` (whose metadata is `m`, not followed) to `dst`, which must not exist.
fn copy_tree(src: &Path, dst: &Path, m: &fs::Metadata) -> io::Result<()> {
    match kind_of(m) {
        ItemKind::File => {
            fs::copy(src, dst)?;
        }
        ItemKind::Symlink => std::os::unix::fs::symlink(fs::read_link(src)?, dst)?,
        ItemKind::Directory => {
            fs::create_dir(dst)?;
            for name in read_names(src)? {
                let from = src.join(&name);
                let child = fs::symlink_metadata(&from)?;
                copy_tree(&from, &dst.join(&name), &child)?;
            }
            // Permissions last, so a read-only directory still takes its children.
            fs::set_permissions(dst, m.permissions())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh scratch directory under the system temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let p = std::env::temp_dir().join(format!("bifrost-fm-{}-{}", std::process::id(), n));
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
}
