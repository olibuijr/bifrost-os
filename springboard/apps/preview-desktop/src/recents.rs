//! The File > Open Recent list: the most recently opened files, newest first, kept as one path per line under the
//! app's data directory. Files chosen in the open panel are copied into the data directory first (the app is handed an
//! open file, not a path), so a recent entry is always a path the app can read again. A copy is removed once it is
//! neither a recent entry nor the open document (`sweep_copies`).

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
    keep_copy_at(data_dir, &copy_name(name), bytes)
}

/// Writes `bytes` as the working copy `place` under `opened/`: a file name, or a numbered folder and a file name for a
/// second file of the same name (`origin::Origins::place`). Returns the copy's path.
pub fn keep_copy_at(data_dir: &Path, place: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let target = data_dir.join("opened").join(place);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&target, bytes)?;
    Ok(target)
}

/// Removes the working copies that nothing refers to any more: every file under `opened/` that is neither an Open
/// Recent entry (a recent file is opened again from its copy) nor `keep`, the document that is open. So the folder
/// holds at most the recent files and the open one, instead of every file ever opened. Returns how many were removed.
/// The numbered folders (the copies of further files of one name) are swept the same way and go once they are empty.
pub fn sweep_copies(data_dir: &Path, recents: &Recents, keep: Option<&Path>) -> usize {
    if data_dir.as_os_str().is_empty() {
        return 0;
    }
    let files = |dir: &Path| std::fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
    let mut removed = 0;
    for path in files(&data_dir.join("opened")) {
        let inner = if path.is_dir() { files(&path) } else { vec![path.clone()] };
        for file in inner {
            if keep == Some(file.as_path()) || recents.paths().contains(&file) {
                continue;
            }
            if std::fs::remove_file(&file).is_ok() {
                removed += 1;
            }
        }
        if path.is_dir() {
            // only an empty folder goes
            let _ = std::fs::remove_dir(&path);
        }
    }
    removed
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
    fn copies_go_once_they_are_neither_recent_nor_open() {
        let dir = std::env::temp_dir().join(format!("viewer-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = keep_copy(&dir, "weeks-old.png", b"x").unwrap();
        let recent = keep_copy(&dir, "recent.pdf", b"x").unwrap();
        let open = keep_copy(&dir, "open.svg", b"x").unwrap();
        let mut r = Recents::default();
        r.add(&recent);
        // the copy that no recent entry names and that is not open goes
        assert_eq!(sweep_copies(&dir, &r, Some(&open)), 1);
        assert!(!old.exists() && recent.exists() && open.exists());
        // the open document was replaced and Open Recent was cleared: its copy and the recent one go too
        r.clear();
        assert_eq!(sweep_copies(&dir, &r, None), 2);
        assert!(!recent.exists() && !open.exists());
        assert_eq!(sweep_copies(Path::new(""), &r, None), 0, "no data folder yet, so nothing is looked at");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_copies_of_further_files_of_one_name_are_swept_with_their_folders() {
        let dir = std::env::temp_dir().join(format!("viewer-sweep-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = keep_copy(&dir, "a.png", b"x").unwrap();
        let second = keep_copy_at(&dir, "2/a.png", b"y").unwrap();
        let third = keep_copy_at(&dir, "3/a.png", b"z").unwrap();
        assert_eq!(second, dir.join("opened/2/a.png"));
        assert_eq!(std::fs::read(&first).unwrap(), b"x", "the first file's copy is not written over");
        let mut r = Recents::default();
        r.add(&second);
        assert_eq!(sweep_copies(&dir, &r, Some(&first)), 1);
        assert!(first.exists() && second.exists() && !third.exists());
        assert!(!dir.join("opened/3").exists() && dir.join("opened/2").exists(), "an emptied folder goes, a used one stays");
        let _ = std::fs::remove_dir_all(&dir);
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
