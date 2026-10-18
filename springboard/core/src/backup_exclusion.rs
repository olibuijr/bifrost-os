//! Backup exclusion (task theme-28): a per-item flag that keeps a file or folder out of the system backup. The set
//! holds normalized absolute paths and reads nothing from disk. The flag applies to the item itself and is not
//! inherited by children. BifrOSt's backup service does not read this set yet.

use std::collections::BTreeSet;

/// Why a path was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    /// The path does not start with `/`, which includes the empty path.
    NotAbsolute(String),
}

/// The set of items excluded from backup, keyed by normalized path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExclusionSet {
    items: BTreeSet<String>,
}

/// Lexically normalizes an absolute path: empty and `.` components are dropped, `..` removes the previous component
/// (and stays at the root), and a trailing slash is removed. Nothing is resolved on disk, so symbolic links are not
/// followed.
pub fn normalize(path: &str) -> Result<String, BackupError> {
    if !path.starts_with('/') {
        return Err(BackupError::NotAbsolute(path.to_string()));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Ok("/".to_string());
    }
    let mut out = String::new();
    for part in parts {
        out.push('/');
        out.push_str(part);
    }
    Ok(out)
}

impl ExclusionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets or clears the exclusion of one item.
    pub fn set_excluded(&mut self, path: &str, excluded: bool) -> Result<(), BackupError> {
        let key = normalize(path)?;
        if excluded {
            self.items.insert(key);
        } else {
            self.items.remove(&key);
        }
        Ok(())
    }

    /// Whether this exact item is excluded. Items beneath an excluded folder are not reported as excluded.
    pub fn is_excluded(&self, path: &str) -> Result<bool, BackupError> {
        Ok(self.items.contains(&normalize(path)?))
    }

    /// Every excluded item, sorted by path.
    pub fn excluded_paths(&self) -> Vec<&str> {
        self.items.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluding_an_item_and_clearing_it_round_trips() {
        let mut set = ExclusionSet::new();
        set.set_excluded("/home/u/Music", true).unwrap();
        assert_eq!(set.is_excluded("/home/u/Music"), Ok(true));
        set.set_excluded("/home/u/Music", false).unwrap();
        assert_eq!(set.is_excluded("/home/u/Music"), Ok(false));
    }

    #[test]
    fn trailing_slash_and_dot_components_name_the_same_item() {
        let mut set = ExclusionSet::new();
        set.set_excluded("/home/u/./Music/", true).unwrap();
        assert_eq!(set.is_excluded("/home/u/Music"), Ok(true));
        assert_eq!(set.excluded_paths(), vec!["/home/u/Music"]);
    }

    #[test]
    fn dot_dot_resolves_lexically_and_stops_at_the_root() {
        assert_eq!(normalize("/home/u/x/../Music").unwrap(), "/home/u/Music");
        assert_eq!(normalize("/../etc").unwrap(), "/etc");
        assert_eq!(normalize("/").unwrap(), "/");
        assert_eq!(normalize("//a//b").unwrap(), "/a/b");
    }

    #[test]
    fn relative_and_empty_paths_are_rejected() {
        assert_eq!(normalize("Music"), Err(BackupError::NotAbsolute("Music".to_string())));
        assert_eq!(normalize(""), Err(BackupError::NotAbsolute(String::new())));
        let mut set = ExclusionSet::new();
        assert!(set.set_excluded("rel/path", true).is_err());
        assert!(set.is_excluded("").is_err());
        assert!(set.excluded_paths().is_empty());
    }

    #[test]
    fn exclusion_applies_to_the_item_only() {
        let mut set = ExclusionSet::new();
        set.set_excluded("/a/b", true).unwrap();
        assert_eq!(set.is_excluded("/a/b/c"), Ok(false));
        assert_eq!(set.is_excluded("/a"), Ok(false));
        assert_eq!(set.is_excluded("/a/b"), Ok(true));
    }

    #[test]
    fn excluded_paths_come_back_sorted() {
        let mut set = ExclusionSet::new();
        for p in ["/z", "/a", "/m"] {
            set.set_excluded(p, true).unwrap();
        }
        assert_eq!(set.excluded_paths(), vec!["/a", "/m", "/z"]);
    }
}

/// The set as text: one normalized absolute path per line, in path order. The shell stores this text in the backup
/// folder and reads it back with [`from_text`].
pub fn to_text(set: &ExclusionSet) -> String {
    let mut out = String::new();
    for path in set.excluded_paths() {
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// Reads the text written by [`to_text`]. A line that is not an absolute path (a blank line included) is skipped, so
/// a damaged line never excludes an item the user did not choose.
pub fn from_text(text: &str) -> ExclusionSet {
    let mut set = ExclusionSet::new();
    for line in text.lines() {
        let _ = set.set_excluded(line.trim(), true);
    }
    set
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn text_round_trip_keeps_only_valid_absolute_paths() {
        let mut set = ExclusionSet::new();
        set.set_excluded("/a/b/", true).unwrap();
        set.set_excluded("/c", true).unwrap();
        let text = to_text(&set);
        assert_eq!(text, "/a/b\n/c\n");
        let back = from_text(&format!("{text}relative/x\n\n"));
        assert_eq!(back, set);
    }
}
