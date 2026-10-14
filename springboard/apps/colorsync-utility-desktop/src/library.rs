//! Finds the ICC profile files: the system folders and the user's folders. Only directory entries are read here; the
//! bytes of a profile are read by the window when the profile is selected.

use std::path::{Path, PathBuf};

/// Where a profile is installed: for every user (system) or for the signed-in user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    System,
    User,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::System => "System",
            Group::User => "User",
        }
    }
}

/// A profile file that was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub group: Group,
}

/// How many folder levels below a profile folder are searched.
const MAX_DEPTH: usize = 4;
/// The extensions profile files use (any letter case).
const EXTENSIONS: [&str; 2] = ["icc", "icm"];

/// The folders searched and the group of each: the system folders first, then the folders of the user at `home`.
pub fn folders(home: &Path) -> Vec<(Group, PathBuf)> {
    vec![
        (Group::System, PathBuf::from("/usr/share/color/icc")),
        (Group::System, PathBuf::from("/usr/local/share/color/icc")),
        (Group::User, home.join(".local/share/color/icc")),
        (Group::User, home.join(".color/icc")),
    ]
}

/// Every profile file in every folder of `folders`, grouped in the order the folders are given.
pub fn scan_all(home: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    for (group, dir) in folders(home) {
        let mut found = Vec::new();
        walk(&dir, group, 0, &mut found);
        found.sort_by(|a, b| a.path.cmp(&b.path));
        out.extend(found);
    }
    out
}

/// True when a file name has a profile extension and a name before it.
pub fn is_profile_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)),
        None => false,
    }
}

fn walk(dir: &Path, group: Group, depth: usize, out: &mut Vec<Found>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // the depth limit also cuts a symbolic link loop
            if depth + 1 < MAX_DEPTH {
                walk(&path, group, depth + 1, out);
            }
        } else if path.is_file() && path.file_name().and_then(|n| n.to_str()).is_some_and(is_profile_name) {
            out.push(Found { path, group });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_need_an_icc_or_icm_extension() {
        assert!(is_profile_name("sRGB.icc"));
        assert!(is_profile_name("Display.ICM"));
        assert!(!is_profile_name("notes.txt"));
        assert!(!is_profile_name(".icc"));
        assert!(!is_profile_name("icc"));
    }

    #[test]
    fn folders_are_system_then_user() {
        let f = folders(Path::new("/home/someone"));
        assert_eq!(f.len(), 4);
        assert_eq!(f[0], (Group::System, PathBuf::from("/usr/share/color/icc")));
        assert_eq!(f[2], (Group::User, PathBuf::from("/home/someone/.local/share/color/icc")));
        assert_eq!(f[3].1, PathBuf::from("/home/someone/.color/icc"));
    }

    #[test]
    fn group_titles() {
        assert_eq!(Group::System.title(), "System");
        assert_eq!(Group::User.title(), "User");
    }
}
