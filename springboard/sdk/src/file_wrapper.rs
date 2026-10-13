//! `FileWrapper`: a regular file, a directory of wrappers (packages such as rtfd or bundles) or
//! a symbolic link held in memory, read from and written to disk.
//!
//! Writes are atomic per top-level item: the tree is written to a sibling temporary path and
//! renamed over the target. Symbolic links read as links and are written back as links.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    File(Vec<u8>),
    Dir(BTreeMap<String, FileWrapper>),
    Link(PathBuf),
}

/// One node of a file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileWrapper {
    node: Node,
    pub preferred_filename: Option<String>,
}

fn unique_key(m: &BTreeMap<String, FileWrapper>, base: &str) -> String {
    let mut key = base.to_string();
    let mut n = 0;
    while m.contains_key(&key) {
        n += 1;
        key = format!("{base} {n}");
    }
    key
}

impl FileWrapper {
    /// `init(regularFileWithContents:)`.
    pub fn regular_file_with_contents(data: Vec<u8>) -> FileWrapper {
        FileWrapper { node: Node::File(data), preferred_filename: None }
    }

    /// `init(directoryWithFileWrappers:)`; each wrapper's key is its filename.
    pub fn directory_with_file_wrappers(children: BTreeMap<String, FileWrapper>) -> FileWrapper {
        let children = children
            .into_iter()
            .map(|(name, mut w)| {
                w.preferred_filename = Some(name.clone());
                (name, w)
            })
            .collect();
        FileWrapper { node: Node::Dir(children), preferred_filename: None }
    }

    /// `init(url:options:)`: reads a file, directory tree or symbolic link.
    pub fn from_path(path: &Path) -> io::Result<FileWrapper> {
        let meta = std::fs::symlink_metadata(path)?;
        let node = if meta.file_type().is_symlink() {
            Node::Link(std::fs::read_link(path)?)
        } else if meta.is_dir() {
            let mut m = BTreeMap::new();
            for e in std::fs::read_dir(path)? {
                let e = e?;
                let name = e.file_name().to_string_lossy().into_owned();
                m.insert(name, FileWrapper::from_path(&e.path())?);
            }
            Node::Dir(m)
        } else {
            Node::File(std::fs::read(path)?)
        };
        let preferred_filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
        Ok(FileWrapper { node, preferred_filename })
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.node, Node::Dir(_))
    }

    pub fn is_regular_file(&self) -> bool {
        matches!(self.node, Node::File(_))
    }

    pub fn is_symbolic_link(&self) -> bool {
        matches!(self.node, Node::Link(_))
    }

    /// `regularFileContents`.
    pub fn regular_file_contents(&self) -> Option<&[u8]> {
        match &self.node {
            Node::File(d) => Some(d),
            _ => None,
        }
    }

    /// `fileWrappers`: the children of a directory, keyed by filename.
    pub fn file_wrappers(&self) -> Option<&BTreeMap<String, FileWrapper>> {
        match &self.node {
            Node::Dir(m) => Some(m),
            _ => None,
        }
    }

    /// `addRegularFile(withContents:preferredFilename:)`; returns the key used. A clashing name
    /// gets a numeric suffix. Returns `None` on a non-directory.
    pub fn add_regular_file(&mut self, contents: Vec<u8>, preferred_filename: &str) -> Option<String> {
        let mut w = FileWrapper::regular_file_with_contents(contents);
        w.preferred_filename = Some(preferred_filename.to_string());
        self.add_file_wrapper(w)
    }

    /// `addFileWrapper(_:)`: adds under its preferred filename (or "untitled").
    pub fn add_file_wrapper(&mut self, mut wrapper: FileWrapper) -> Option<String> {
        let Node::Dir(m) = &mut self.node else { return None };
        let base = wrapper.preferred_filename.clone().unwrap_or_else(|| "untitled".into());
        let key = unique_key(m, &base);
        wrapper.preferred_filename = Some(key.clone());
        m.insert(key.clone(), wrapper);
        Some(key)
    }

    /// `removeFileWrapper(_:)` by key.
    pub fn remove_file_wrapper(&mut self, key: &str) -> Option<FileWrapper> {
        match &mut self.node {
            Node::Dir(m) => m.remove(key),
            _ => None,
        }
    }

    /// `write(to:options:originalContentsURL:)` with atomic writing: the tree lands in a sibling
    /// temporary path and replaces `path` by rename.
    pub fn write_to(&self, path: &Path) -> io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no filename"))?;
        let tmp = path.with_file_name(format!(".{}.writing-{}", name.to_string_lossy(), std::process::id()));
        remove_any(&tmp)?;
        if let Err(e) = self.write_tree(&tmp) {
            let _ = remove_any(&tmp);
            return Err(e);
        }
        remove_any(path)?;
        std::fs::rename(&tmp, path)
    }

    fn write_tree(&self, path: &Path) -> io::Result<()> {
        match &self.node {
            Node::File(d) => std::fs::write(path, d),
            Node::Link(t) => std::os::unix::fs::symlink(t, path),
            Node::Dir(m) => {
                std::fs::create_dir(path)?;
                m.iter().try_for_each(|(k, w)| w.write_tree(&path.join(k)))
            }
        }
    }

    /// `matchesContents(of:)`: whether the item at `path` has the same tree and bytes.
    pub fn matches_contents(&self, path: &Path) -> bool {
        FileWrapper::from_path(path).map(|d| d.node == self.node).unwrap_or(false)
    }

    /// `needsToBeUpdated(fromPath:)`: true when the disk differs from this wrapper.
    pub fn needs_to_be_updated(&self, path: &Path) -> bool {
        !self.matches_contents(path)
    }
}

fn remove_any(p: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "A").unwrap();
        fs::write(root.join("sub/b.txt"), "B").unwrap();
    }

    #[test]
    fn reads_a_directory_tree() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("doc.rtfd");
        tree(&root);
        let w = FileWrapper::from_path(&root).unwrap();
        assert!(w.is_directory() && !w.is_regular_file());
        let kids = w.file_wrappers().unwrap();
        assert_eq!(kids["a.txt"].regular_file_contents(), Some(&b"A"[..]));
        assert_eq!(kids["sub"].file_wrappers().unwrap()["b.txt"].regular_file_contents(), Some(&b"B"[..]));
    }

    #[test]
    fn add_file_then_write_atomically_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("doc.rtfd");
        tree(&root);
        let mut w = FileWrapper::from_path(&root).unwrap();
        assert_eq!(w.add_regular_file(b"C".to_vec(), "c.txt").as_deref(), Some("c.txt"));
        assert_eq!(w.add_regular_file(b"D".to_vec(), "c.txt").as_deref(), Some("c.txt 1"));
        w.write_to(&root).unwrap();
        assert_eq!(fs::read(root.join("c.txt")).unwrap(), b"C");
        assert!(w.matches_contents(&root));
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1, "no temporary left behind");
    }

    #[test]
    fn matches_contents_detects_change() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("p");
        tree(&root);
        let w = FileWrapper::from_path(&root).unwrap();
        assert!(!w.needs_to_be_updated(&root));
        fs::write(root.join("sub/b.txt"), "changed").unwrap();
        assert!(w.needs_to_be_updated(&root));
        assert!(w.needs_to_be_updated(&d.path().join("missing")));
    }

    #[test]
    fn builds_in_memory_and_writes() {
        let d = tempfile::tempdir().unwrap();
        let mut kids = BTreeMap::new();
        kids.insert("x".to_string(), FileWrapper::regular_file_with_contents(b"1".to_vec()));
        let w = FileWrapper::directory_with_file_wrappers(kids);
        let out = d.path().join("pkg");
        w.write_to(&out).unwrap();
        assert_eq!(fs::read(out.join("x")).unwrap(), b"1");
        let f = FileWrapper::regular_file_with_contents(b"z".to_vec());
        let one = d.path().join("one");
        f.write_to(&one).unwrap();
        assert!(f.is_regular_file() && f.matches_contents(&one));
    }

    #[test]
    fn symlinks_are_kept_as_links() {
        let d = tempfile::tempdir().unwrap();
        let l = d.path().join("l");
        std::os::unix::fs::symlink("target", &l).unwrap();
        let w = FileWrapper::from_path(&l).unwrap();
        assert!(w.is_symbolic_link());
        let out = d.path().join("l2");
        w.write_to(&out).unwrap();
        assert_eq!(fs::read_link(&out).unwrap(), PathBuf::from("target"));
    }
}
