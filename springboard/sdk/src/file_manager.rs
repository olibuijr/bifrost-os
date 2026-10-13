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
//! Gaps: no `FileWrapper` (bundles of files), no bookmarks or security-scoped URLs (user files go through the file
//! portal grants in `docbroker`), no attributes beyond kind, size and modification time, no extended attributes,
//! no creation of symbolic links, no copy progress or delegate callbacks, and no cloud-synced (ubiquitous) items.
//! `replace_item_at` has no `usingNewMetadataOnly` or `withoutDeletingBackupItem` options. The enumerator reads the
//! whole tree when it is created and returns no resource keys. The user directories are the XDG-style Linux
//! locations (`$HOME/Documents`, `$XDG_DATA_HOME`, `$XDG_CACHE_HOME`), not the `~/Library` folders.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// `FileManager.DirectoryEnumerator`: the full paths of the items below a directory, in order.
#[derive(Debug)]
pub struct DirectoryEnumerator {
    items: std::vec::IntoIter<PathBuf>,
}

impl Iterator for DirectoryEnumerator {
    type Item = PathBuf;

    fn next(&mut self) -> Option<PathBuf> {
        self.items.next()
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
            .enumerator(dir, DirectoryEnumerationOptions::default())?
            .filter_map(|p| p.strip_prefix(dir).ok().map(Path::to_path_buf))
            .collect())
    }

    /// `enumerator(at:includingPropertiesForKeys:options:)`: the full paths of the items below `at`, depth first,
    /// names sorted within each level. The tree is read now; an unreadable directory is an error.
    pub fn enumerator(&self, at: &Path, options: DirectoryEnumerationOptions) -> io::Result<DirectoryEnumerator> {
        let mut items = Vec::new();
        collect_tree(at, options, &mut items)?;
        Ok(DirectoryEnumerator { items: items.into_iter() })
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

    /// `replaceItemAt(_:withItemAt:backupItemName:options:)`: moves `new_item` into the place of `original` and
    /// returns that place. With `backup_item_name`, the old item is kept beside it under that name, replacing an older
    /// backup of that name. Without one, the old item is deleted once the new one is in place. If the new item
    /// cannot be moved, the old item is put back and the error is returned.
    pub fn replace_item_at(&self, original: &Path, new_item: &Path, backup_item_name: Option<&str>) -> io::Result<PathBuf> {
        if self.item_kind(original).is_none() {
            return Err(not_found(original));
        }
        if self.item_kind(new_item).is_none() {
            return Err(not_found(new_item));
        }
        let dir = parent_dir(original);
        let backup = match backup_item_name {
            Some(name) => dir.join(name),
            None => {
                let name = original.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                dir.join(format!(".{name}.replaced-{}", std::process::id()))
            }
        };
        if self.item_kind(&backup).is_some() {
            self.remove_item(&backup)?;
        }
        fs::rename(original, &backup)?;
        if let Err(e) = self.move_item(new_item, original) {
            let _ = fs::rename(&backup, original);
            return Err(e);
        }
        if backup_item_name.is_none() {
            self.remove_item(&backup)?;
        }
        Ok(original.to_path_buf())
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
                if let Err(e) = copy_tree(src, dst, &m) {
                    let _ = self.remove_item(dst);
                    return Err(e);
                }
                self.remove_item(src)
            }
            Err(e) => Err(e),
        }
    }
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

/// Pre-order walk that honours `options`. A symbolic link is listed but never entered.
fn collect_tree(dir: &Path, options: DirectoryEnumerationOptions, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for name in read_names(dir)? {
        if options.skips_hidden_files && is_hidden(&name) {
            continue;
        }
        let child = dir.join(&name);
        let is_real_dir = kind_of(&fs::symlink_metadata(&child)?) == ItemKind::Directory;
        out.push(child.clone());
        if is_real_dir && !options.skips_subdirectory_descendants {
            collect_tree(&child, options, out)?;
        }
    }
    Ok(())
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
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

fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} does not exist", path.display()))
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
        let list = |o| relative(&s.0, fm.enumerator(&s.0, o).unwrap());

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
        let first = fm.enumerator(&s.0, DirectoryEnumerationOptions::default()).unwrap().next().unwrap();
        assert_eq!(first, s.0.join("a.txt"));
        assert!(fm.enumerator(&s.0.join("missing"), DirectoryEnumerationOptions::default()).is_err());
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
            .replace_item_at(&s.0.join("doc.txt"), &s.0.join("new.txt"), Some("doc.bak"))
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
        NativeFileManager.replace_item_at(&s.0.join("doc.txt"), &s.0.join("new.txt"), None).unwrap();
        assert_eq!(fs::read_to_string(s.0.join("doc.txt")).unwrap(), "new");
        let names: Vec<OsString> = fs::read_dir(&s.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("doc.txt")], "the old item is deleted");
    }

    #[test]
    fn replace_item_at_needs_both_items_and_changes_nothing_when_one_is_missing() {
        let s = Scratch::new();
        touch(&s.0.join("doc.txt"), "old");
        let fm = NativeFileManager;
        let err = fm.replace_item_at(&s.0.join("doc.txt"), &s.0.join("absent.txt"), None).unwrap_err();
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
