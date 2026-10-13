//! `FileWrapper` parity (Foundation, "File System"): a file, a directory of files or a symbolic link held in memory,
//! so a document can be a package (a directory that looks like one file) and be saved as a whole.
//!
//! - Three kinds: regular file (its bytes), directory (children by file name) and symbolic link (its destination).
//! - `read_from` loads a tree from disk and remembers where each regular file came from. `matches_contents_of`
//!   compares the tree with what is on disk now, so an app can tell whether a document changed outside of it.
//! - `write_to` writes the tree. With `atomic`, it writes next to the target and swaps the result in with
//!   `replace_item_at`, so a failed save leaves the old package whole. With `original_contents`, a regular file that
//!   still matches the same-named file in the original package is hard linked instead of written again (the way
//!   a package with large, unchanged members saves quickly); it falls back to a copy when linking fails.
//! - `with_name_updating` makes the wrapper adopt the written name as its `filename`.
//!
//! Gaps: no serialized representation, no `fileAttributes` beyond the content, and symbolic links are kept as links
//! (never resolved on read).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::file_manager::{NativeFileManager, ReplaceOptions};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// `FileWrapper.WritingOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WritingOptions {
    /// Write to a temporary item first and swap it in, so the target is never half written.
    pub atomic: bool,
    /// Adopt the name that was written as the wrapper's `filename`.
    pub with_name_updating: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Content {
    Regular(Vec<u8>),
    Directory(BTreeMap<String, FileWrapper>),
    Symlink(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileWrapper {
    content: Content,
    /// The name the wrapper would like when it is added to a directory.
    pub preferred_filename: Option<String>,
    /// The name it has in its parent directory, or the name it was last written under.
    pub filename: Option<String>,
    /// Where a regular file was read from, for the hard link shortcut of `write_to`.
    source: Option<PathBuf>,
}

impl FileWrapper {
    fn new(content: Content) -> FileWrapper {
        FileWrapper { content, preferred_filename: None, filename: None, source: None }
    }

    /// `init(regularFileWithContents:)`.
    pub fn regular_file_with_contents(contents: Vec<u8>) -> FileWrapper {
        FileWrapper::new(Content::Regular(contents))
    }

    /// `init(directoryWithFileWrappers:)`: each child takes its key as its file name.
    pub fn directory_with_file_wrappers(children: Vec<(String, FileWrapper)>) -> FileWrapper {
        let mut map = BTreeMap::new();
        for (name, mut child) in children {
            child.filename = Some(name.clone());
            if child.preferred_filename.is_none() {
                child.preferred_filename = Some(name.clone());
            }
            map.insert(name, child);
        }
        FileWrapper::new(Content::Directory(map))
    }

    /// `init(symbolicLinkWithDestinationURL:)`.
    pub fn symbolic_link_with_destination(destination: PathBuf) -> FileWrapper {
        FileWrapper::new(Content::Symlink(destination))
    }

    /// `init(url:options:)`: reads a file, a symbolic link or a whole directory tree.
    pub fn read_from(path: &Path) -> io::Result<FileWrapper> {
        let meta = fs::symlink_metadata(path)?;
        let mut wrapper = if meta.file_type().is_symlink() {
            FileWrapper::new(Content::Symlink(fs::read_link(path)?))
        } else if meta.is_dir() {
            let mut map = BTreeMap::new();
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                map.insert(name, FileWrapper::read_from(&entry.path())?);
            }
            FileWrapper::new(Content::Directory(map))
        } else {
            let mut w = FileWrapper::new(Content::Regular(fs::read(path)?));
            w.source = Some(path.to_path_buf());
            w
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        wrapper.preferred_filename = name.clone();
        wrapper.filename = name;
        Ok(wrapper)
    }

    pub fn is_regular_file(&self) -> bool {
        matches!(self.content, Content::Regular(_))
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.content, Content::Directory(_))
    }

    pub fn is_symbolic_link(&self) -> bool {
        matches!(self.content, Content::Symlink(_))
    }

    /// `regularFileContents`.
    pub fn regular_file_contents(&self) -> Option<&[u8]> {
        if let Content::Regular(b) = &self.content { Some(b) } else { None }
    }

    /// `fileWrappers`: the children of a directory, by file name.
    pub fn file_wrappers(&self) -> Option<&BTreeMap<String, FileWrapper>> {
        if let Content::Directory(d) = &self.content { Some(d) } else { None }
    }

    /// `symbolicLinkDestinationURL`.
    pub fn symbolic_link_destination(&self) -> Option<&Path> {
        if let Content::Symlink(p) = &self.content { Some(p) } else { None }
    }

    /// `addFileWrapper(_:)`: adds a child under its preferred file name, or a unique variant ("name 2.ext") when
    /// that name is taken, and returns the name used. Only a directory takes children, and the child needs a
    /// preferred name.
    pub fn add_file_wrapper(&mut self, mut child: FileWrapper) -> io::Result<String> {
        let Content::Directory(children) = &mut self.content else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "only a directory file wrapper takes children"));
        };
        let wanted = child
            .preferred_filename
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the child has no preferred file name"))?;
        let name = unique_name(children, &wanted);
        child.filename = Some(name.clone());
        children.insert(name.clone(), child);
        Ok(name)
    }

    /// `addRegularFile(withContents:preferredFilename:)`.
    pub fn add_regular_file(&mut self, contents: Vec<u8>, preferred_filename: &str) -> io::Result<String> {
        let mut child = FileWrapper::regular_file_with_contents(contents);
        child.preferred_filename = Some(preferred_filename.to_string());
        self.add_file_wrapper(child)
    }

    /// `removeFileWrapper(_:)`: true when a child of that name existed.
    pub fn remove_file_wrapper(&mut self, name: &str) -> bool {
        match &mut self.content {
            Content::Directory(children) => children.remove(name).is_some(),
            _ => false,
        }
    }

    /// `keys(for:)` / `fileWrappers[name]`: one child of a directory.
    pub fn file_wrapper(&self, name: &str) -> Option<&FileWrapper> {
        self.file_wrappers()?.get(name)
    }

    /// `matchesContents(of:)`: true when the item at `path` has the same kind and the same content as this tree
    /// (children by name, file bytes, link destination).
    pub fn matches_contents_of(&self, path: &Path) -> bool {
        let Ok(meta) = fs::symlink_metadata(path) else { return false };
        match &self.content {
            Content::Symlink(dest) => meta.file_type().is_symlink() && fs::read_link(path).map_or(false, |d| &d == dest),
            Content::Regular(bytes) => {
                meta.file_type().is_file() && meta.len() == bytes.len() as u64 && fs::read(path).map_or(false, |b| &b == bytes)
            }
            Content::Directory(children) => {
                if !meta.is_dir() {
                    return false;
                }
                let Ok(rd) = fs::read_dir(path) else { return false };
                let mut count = 0usize;
                for entry in rd {
                    let Ok(entry) = entry else { return false };
                    count += 1;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    match children.get(&name) {
                        Some(child) if child.matches_contents_of(&entry.path()) => {}
                        _ => return false,
                    }
                }
                count == children.len()
            }
        }
    }

    /// `write(to:options:originalContentsURL:)`.
    ///
    /// An existing item at `path` is replaced: atomically with `options.atomic`, otherwise it is removed first.
    /// `original_contents` names the previous version of this package, whose unchanged files are reused.
    pub fn write_to(&mut self, path: &Path, options: WritingOptions, original_contents: Option<&Path>) -> io::Result<()> {
        let fm = NativeFileManager;
        if fm.file_exists(path) {
            if options.atomic {
                let tmp = temp_beside(path);
                if let Err(e) = self.write_tree(&tmp, original_contents) {
                    let _ = fm.remove_item(&tmp);
                    return Err(e);
                }
                if let Err(e) = fm.replace_item_at(path, &tmp, None, ReplaceOptions::default()) {
                    let _ = fm.remove_item(&tmp);
                    return Err(e);
                }
            } else {
                fm.remove_item(path)?;
                self.write_tree(path, original_contents)?;
            }
        } else {
            self.write_tree(path, original_contents)?;
        }
        if options.with_name_updating {
            self.filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
        }
        Ok(())
    }

    fn write_tree(&self, dest: &Path, original: Option<&Path>) -> io::Result<()> {
        match &self.content {
            Content::Regular(bytes) => {
                if let (Some(orig), Some(src)) = (original, &self.source) {
                    if src == orig && self.matches_contents_of(orig) && fs::hard_link(orig, dest).is_ok() {
                        return Ok(());
                    }
                }
                fs::write(dest, bytes)
            }
            Content::Symlink(target) => std::os::unix::fs::symlink(target, dest),
            Content::Directory(children) => {
                fs::create_dir(dest)?;
                for (name, child) in children {
                    child.write_tree(&dest.join(name), original.map(|o| o.join(name)).as_deref())?;
                }
                Ok(())
            }
        }
    }
}

/// `name`, or `stem N.ext` with the smallest N >= 2 that is free.
fn unique_name(children: &BTreeMap<String, FileWrapper>, name: &str) -> String {
    if !children.contains_key(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| format!("{stem} {n}{ext}")).find(|candidate| !children.contains_key(candidate)).unwrap_or_default()
}

fn temp_beside(path: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.write-{}-{n}", std::process::id()))
}
