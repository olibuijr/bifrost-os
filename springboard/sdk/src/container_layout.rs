//! Per-app container layout: the fixed folders inside an app's data directory (`Documents`, `Library`,
//! `Library/Caches`, `tmp`). The split tells the system what to back up and what it may purge: `Documents`
//! and `Library` are kept, `Library/Caches` and `tmp` may be cleared when space is short.
//!
//! The path rules are pure. `ensure` is the only I/O: it creates the folders and refuses a container root or
//! folder that is a symbolic link, so an app cannot redirect its own container outside the sandbox.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One fixed folder of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    /// User-visible documents the app creates. Backed up, never purged by the system.
    Documents,
    /// Preferences, state and other data the app keeps between launches. Backed up.
    Library,
    /// Re-creatable data. Not backed up, and the system may purge it.
    Caches,
    /// Scratch files for the current launch. The system may purge it at any time.
    Tmp,
}

impl Folder {
    /// Every folder, in creation order (a parent before its children).
    pub const ALL: [Folder; 4] = [Folder::Documents, Folder::Library, Folder::Caches, Folder::Tmp];

    /// The folder path relative to the container root.
    pub fn relative(self) -> &'static str {
        match self {
            Folder::Documents => "Documents",
            Folder::Library => "Library",
            Folder::Caches => "Library/Caches",
            Folder::Tmp => "tmp",
        }
    }

    /// Whether the folder is included in backups.
    pub fn backed_up(self) -> bool {
        matches!(self, Folder::Documents | Folder::Library)
    }

    /// Whether the system may remove the folder's contents to free space.
    pub fn purgeable(self) -> bool {
        matches!(self, Folder::Caches | Folder::Tmp)
    }
}

/// Why a relative path was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute,
    /// A `..`, a Windows prefix or a `.` component, or an empty component.
    Escapes,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Empty => write!(f, "path is empty"),
            PathError::Absolute => write!(f, "path must be relative to the container"),
            PathError::Escapes => write!(f, "path leaves the container"),
        }
    }
}

/// The layout of one app container rooted at `root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerLayout {
    root: PathBuf,
}

impl ContainerLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ContainerLayout { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute path of a fixed folder.
    pub fn folder(&self, folder: Folder) -> PathBuf {
        self.root.join(folder.relative())
    }

    /// Resolves a path given by the app, relative to the container root. Empty paths, absolute paths and any
    /// `..` component are refused, so the result always stays under the root.
    pub fn resolve(&self, relative: &str) -> Result<PathBuf, PathError> {
        if relative.is_empty() {
            return Err(PathError::Empty);
        }
        let path = Path::new(relative);
        let mut out = self.root.clone();
        for component in path.components() {
            match component {
                Component::Normal(part) => out.push(part),
                Component::RootDir | Component::Prefix(_) => return Err(PathError::Absolute),
                Component::CurDir | Component::ParentDir => return Err(PathError::Escapes),
            }
        }
        if out == self.root {
            return Err(PathError::Empty);
        }
        Ok(out)
    }

    /// Creates the fixed folders that do not exist yet. Fails if the root or a folder is a symbolic link, or
    /// exists as a file.
    pub fn ensure(&self) -> io::Result<()> {
        create_checked_dir(&self.root)?;
        for folder in Folder::ALL {
            let mut current = self.root.clone();
            for part in Path::new(folder.relative()).components() {
                if let Component::Normal(name) = part {
                    current.push(name);
                    create_checked_dir(&current)?;
                }
            }
        }
        Ok(())
    }
}

fn create_checked_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(io::Error::other(format!("container folder is a symbolic link: {}", path.display())));
            }
            if !meta.is_dir() {
                return Err(io::Error::other(format!("container folder is not a directory: {}", path.display())));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(path),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout under a fresh temporary directory; the directory is removed when the guard drops.
    fn layout() -> (ContainerLayout, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (ContainerLayout::new(dir.path().join("container")), dir)
    }

    #[test]
    fn folder_paths_and_policy() {
        assert_eq!(Folder::Caches.relative(), "Library/Caches");
        assert!(Folder::Documents.backed_up() && Folder::Library.backed_up());
        assert!(!Folder::Caches.backed_up() && !Folder::Tmp.backed_up());
        assert!(Folder::Caches.purgeable() && Folder::Tmp.purgeable());
        assert!(!Folder::Documents.purgeable() && !Folder::Library.purgeable());
    }

    #[test]
    fn resolve_keeps_paths_inside_the_root() {
        let l = ContainerLayout::new("/data/app");
        assert_eq!(l.resolve("Documents/a.txt").unwrap(), PathBuf::from("/data/app/Documents/a.txt"));
        assert_eq!(l.resolve("Library/Caches/x").unwrap(), PathBuf::from("/data/app/Library/Caches/x"));
        assert_eq!(l.resolve("").unwrap_err(), PathError::Empty);
        assert_eq!(l.resolve("/etc/passwd").unwrap_err(), PathError::Absolute);
        assert_eq!(l.resolve("Documents/../../etc").unwrap_err(), PathError::Escapes);
        assert_eq!(l.resolve("./Documents").unwrap_err(), PathError::Escapes);
    }

    #[test]
    fn ensure_creates_every_folder_once() {
        let (l, _guard) = layout();
        l.ensure().unwrap();
        l.ensure().unwrap();
        for folder in Folder::ALL {
            assert!(l.folder(folder).is_dir(), "{folder:?} missing");
        }
    }

    #[test]
    fn ensure_refuses_a_symlinked_caches_folder() {
        let (l, guard) = layout();
        let outside = guard.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(l.root().join("Library")).unwrap();
        std::os::unix::fs::symlink(&outside, l.root().join("Library/Caches")).unwrap();
        assert!(l.ensure().is_err());
    }

    #[test]
    fn ensure_refuses_a_file_in_place_of_a_folder() {
        let (l, _guard) = layout();
        fs::create_dir_all(l.root()).unwrap();
        fs::write(l.root().join("tmp"), b"x").unwrap();
        assert!(l.ensure().is_err());
    }
}
