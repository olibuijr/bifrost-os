//! Backup exclusion: the items the system backup skips, and whether a path falls under one of them (std only).
//!
//! An excluded item is a folder or a file given by its absolute path. Excluding a folder excludes everything
//! under it, and the match is by whole path component, so `/home/u/cache` does not cover `/home/u/cache2`.
//! The check is lexical: it does not touch the disk, so the caller passes paths without `.` or `..` parts.
//! The set keeps its paths sorted, and `to_lines` / `load_lines` store it one path per line for the prefs file.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Why a path was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupError {
    /// Only absolute paths can be excluded, so a relative path could never match a real item.
    NotAbsolute,
    /// The path contains a line break, which the one-path-per-line store cannot hold.
    LineBreak,
}

/// The set of excluded items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackupExclusions {
    roots: BTreeSet<PathBuf>,
}

impl BackupExclusions {
    pub fn new() -> Self {
        BackupExclusions::default()
    }

    /// Excludes the item at `path` (`excluded` true) or includes it again (false). Including an item that is not
    /// excluded does nothing, and an item below an excluded folder is still excluded by that folder.
    pub fn set_excluded(&mut self, path: &Path, excluded: bool) -> Result<(), BackupError> {
        if !path.is_absolute() {
            return Err(BackupError::NotAbsolute);
        }
        if path.to_string_lossy().contains('\n') || path.to_string_lossy().contains('\r') {
            return Err(BackupError::LineBreak);
        }
        if excluded {
            self.roots.insert(path.to_path_buf());
        } else {
            self.roots.remove(path);
        }
        Ok(())
    }

    /// Whether `path` is excluded itself or sits under an excluded folder.
    pub fn is_excluded(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }

    /// Number of separately excluded items (a folder and its contents count once, as the folder).
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// One excluded path per line, sorted.
    pub fn to_lines(&self) -> String {
        self.roots.iter().map(|p| format!("{}\n", p.display())).collect()
    }

    /// Reads `to_lines` output. Blank lines and lines that are not absolute paths are skipped.
    pub fn load_lines(&mut self, text: &str) {
        for line in text.lines() {
            let path = Path::new(line);
            if path.is_absolute() {
                self.roots.insert(path.to_path_buf());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_excluded_folder_covers_its_contents_by_whole_component() {
        let mut ex = BackupExclusions::new();
        ex.set_excluded(Path::new("/home/u/cache"), true).unwrap();
        assert!(ex.is_excluded(Path::new("/home/u/cache")));
        assert!(ex.is_excluded(Path::new("/home/u/cache/x.bin")));
        assert!(ex.is_excluded(Path::new("/home/u/cache/deep/y")));
        assert!(!ex.is_excluded(Path::new("/home/u/cache2/x")), "a shared prefix is not a parent");
        assert!(!ex.is_excluded(Path::new("/home/u")), "a parent of the folder is not excluded");
        assert!(!ex.is_excluded(Path::new("home/u/cache")), "relative paths never match");
    }

    #[test]
    fn including_an_item_again_removes_it() {
        let mut ex = BackupExclusions::new();
        ex.set_excluded(Path::new("/srv/work"), true).unwrap();
        ex.set_excluded(Path::new("/srv/work/keep"), true).unwrap();
        assert_eq!(ex.len(), 2);
        ex.set_excluded(Path::new("/srv/work"), false).unwrap();
        assert!(!ex.is_excluded(Path::new("/srv/work/a")));
        assert!(ex.is_excluded(Path::new("/srv/work/keep/a")), "its own exclusion stays");
        assert_eq!(ex.len(), 1);
        ex.set_excluded(Path::new("/srv/never"), false).unwrap();
        assert_eq!(ex.len(), 1, "including an item that was never excluded changes nothing");
    }

    #[test]
    fn relative_paths_and_line_breaks_are_refused() {
        let mut ex = BackupExclusions::new();
        assert_eq!(ex.set_excluded(Path::new("docs/a"), true), Err(BackupError::NotAbsolute));
        assert_eq!(ex.set_excluded(Path::new("/a\nb"), true), Err(BackupError::LineBreak));
        assert_eq!(ex.set_excluded(Path::new("/a\rb"), true), Err(BackupError::LineBreak));
        assert!(ex.is_empty());
    }

    #[test]
    fn exclusions_round_trip_sorted_one_per_line() {
        let mut ex = BackupExclusions::new();
        ex.set_excluded(Path::new("/b"), true).unwrap();
        ex.set_excluded(Path::new("/a/c"), true).unwrap();
        let text = ex.to_lines();
        assert_eq!(text, "/a/c\n/b\n");
        let mut fresh = BackupExclusions::new();
        fresh.load_lines(&(text + "\ngarbage\n/d\n"));
        assert_eq!(fresh, ex_with(&["/a/c", "/b", "/d"]));
        assert!(fresh.is_excluded(Path::new("/d/e")));
    }

    #[test]
    fn a_top_level_folder_covers_everything_under_it_only() {
        let mut ex = BackupExclusions::new();
        ex.set_excluded(Path::new("/var"), true).unwrap();
        assert!(ex.is_excluded(Path::new("/var/lib/app/data")));
        assert!(!ex.is_excluded(Path::new("/")), "the root is never covered by a folder below it");
        assert!(!ex.is_excluded(Path::new("/vari")));
    }

    fn ex_with(paths: &[&str]) -> BackupExclusions {
        let mut ex = BackupExclusions::new();
        for p in paths {
            ex.set_excluded(Path::new(p), true).unwrap();
        }
        ex
    }
}
