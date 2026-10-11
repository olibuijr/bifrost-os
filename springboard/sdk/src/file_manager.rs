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
//! Gaps: no `FileWrapper` (bundles of files), no bookmarks or security-scoped URLs (user files go through the file
//! portal grants in `docbroker`), no attributes beyond kind, size and modification time, no extended attributes,
//! no creation of symbolic links, no copy progress or delegate callbacks, and no cloud-synced (ubiquitous) items.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
