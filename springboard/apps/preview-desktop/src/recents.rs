//! The File > Open Recent list: the most recently opened files, newest first, kept as one path per line under the
//! app's data directory. Files chosen in the open panel are copied into the data directory first (the app is handed an
//! open file, not a path), so a recent entry is always a path the app can read again.

use std::path::{Path, PathBuf};

/// The most entries Open Recent keeps.
pub const MAX_RECENTS: usize = 10;

/// The recently opened files, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recents {
    paths: Vec<PathBuf>,
}

impl Recents {
    /// Reads the list from its text: one path per line, newest first. Blank lines and repeats are dropped.
    pub fn parse(text: &str) -> Recents {
        let mut recents = Recents::default();
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        for line in lines.into_iter().rev() {
            recents.add(Path::new(line));
        }
        recents
    }

    /// The list as text, newest first.
    pub fn to_text(&self) -> String {
        self.paths.iter().map(|p| format!("{}\n", p.display())).collect()
    }

    /// Notes that `path` was just opened: it goes first, and an older entry for it goes away.
    pub fn add(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENTS);
    }

    /// Forgets every entry (Open Recent > Clear Menu).
    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Drops entries whose file no longer exists.
    pub fn prune(&mut self) {
        self.paths.retain(|p| p.exists());
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The entry at `index`, newest first.
    pub fn get(&self, index: usize) -> Option<&Path> {
        self.paths.get(index).map(PathBuf::as_path)
    }
}

/// The file name a copy of the chosen file `name` is kept under: only its last component, never empty.
pub fn copy_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if last.is_empty() || last == "." || last == ".." {
        "Untitled".into()
    } else {
        last.to_string()
    }
}

/// Where the recents list is kept.
pub fn list_path(data_dir: &Path) -> PathBuf {
    data_dir.join("recents.txt")
}

/// Where the copy of an opened file named `name` is kept.
pub fn copy_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join("opened").join(copy_name(name))
}

/// Loads the list; a missing or unreadable file gives an empty one.
pub fn load(data_dir: &Path) -> Recents {
    let mut recents = std::fs::read_to_string(list_path(data_dir)).map(|t| Recents::parse(&t)).unwrap_or_default();
    recents.prune();
    recents
}

/// Saves the list.
pub fn save(data_dir: &Path, recents: &Recents) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(list_path(data_dir), recents.to_text())
}

/// Writes `bytes` into the data directory as the opened file `name` and returns the copy's path.
pub fn keep_copy(data_dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let target = copy_path(data_dir, name);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&target, bytes)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_file_goes_first_and_repeats_move_up() {
        let mut r = Recents::default();
        r.add(Path::new("/a"));
        r.add(Path::new("/b"));
        r.add(Path::new("/a"));
        assert_eq!(r.paths(), [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn the_list_is_capped() {
        let mut r = Recents::default();
        for i in 0..MAX_RECENTS + 3 {
            r.add(Path::new(&format!("/f{i}")));
        }
        assert_eq!(r.paths().len(), MAX_RECENTS);
        assert_eq!(r.get(0), Some(Path::new(&format!("/f{}", MAX_RECENTS + 2))));
    }

    #[test]
    fn text_round_trips_in_order() {
        let mut r = Recents::default();
        r.add(Path::new("/one.pdf"));
        r.add(Path::new("/two.png"));
        assert_eq!(Recents::parse(&r.to_text()), r);
        assert_eq!(Recents::parse("\n\n/x\n/x\n"), Recents { paths: vec![PathBuf::from("/x")] });
    }

    #[test]
    fn clear_empties_the_list() {
        let mut r = Recents::parse("/a\n/b\n");
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn a_copy_keeps_only_the_last_name_component() {
        assert_eq!(copy_name("../../etc/passwd"), "passwd");
        assert_eq!(copy_name("pic.png"), "pic.png");
        assert_eq!(copy_name(".."), "Untitled");
        assert_eq!(copy_name(""), "Untitled");
    }

    #[test]
    fn keep_copy_writes_under_the_data_dir_and_load_prunes_missing_files() {
        let dir = std::env::temp_dir().join(format!("viewer-recents-{}", std::process::id()));
        let path = keep_copy(&dir, "a/b.png", b"x").unwrap();
        assert_eq!(path, dir.join("opened").join("b.png"));
        let mut r = Recents::default();
        r.add(&path);
        r.add(&dir.join("opened").join("gone.png"));
        save(&dir, &r).unwrap();
        assert_eq!(load(&dir).paths(), [path]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
