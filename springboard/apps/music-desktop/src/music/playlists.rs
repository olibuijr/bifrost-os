//! Playlists kept as plain files: `<dir>/<name>.m3u`, one song path per
//! line (a `#EXTM3U` header first). Songs are kept by path so a rescan of
//! the library does not break them. Pure file logic, no UI.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub name: String,
    pub songs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct Playlists {
    dir: PathBuf,
    lists: Vec<Playlist>,
}

/// A name that is safe as a file name: no separators or control characters,
/// trimmed, not empty and not a dot name.
pub fn clean_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control() && !"/\\:*?\"<>|".contains(*c)).collect();
    let n = n.trim().trim_start_matches('.').trim().to_string();
    (!n.is_empty()).then(|| n.chars().take(60).collect())
}

impl Playlists {
    /// Reads every playlist in `dir` (which may not exist yet), by name.
    pub fn open(dir: PathBuf) -> Self {
        let mut lists = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "m3u") {
                    if let (Some(name), Ok(text)) = (p.file_stem().map(|s| s.to_string_lossy().into_owned()), std::fs::read_to_string(&p)) {
                        let songs = text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(PathBuf::from).collect();
                        lists.push(Playlist { name, songs });
                    }
                }
            }
        }
        lists.sort_by_key(|l| l.name.to_lowercase());
        Playlists { dir, lists }
    }

    pub fn lists(&self) -> &[Playlist] {
        &self.lists
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.m3u"))
    }

    fn save(&self, i: usize) {
        let Some(l) = self.lists.get(i) else { return };
        let _ = std::fs::create_dir_all(&self.dir);
        let mut text = String::from("#EXTM3U\n");
        for p in &l.songs {
            text.push_str(&format!("{}\n", p.display()));
        }
        let tmp = self.dir.join(format!(".{}.tmp", l.name));
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, self.file(&l.name));
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.lists.iter().position(|l| l.name.to_lowercase() == name.to_lowercase())
    }

    /// Makes an empty playlist; the index it lands at, or None when the name
    /// is unusable or taken.
    pub fn create(&mut self, name: &str) -> Option<usize> {
        let name = clean_name(name)?;
        if self.position(&name).is_some() {
            return None;
        }
        self.lists.push(Playlist { name: name.clone(), songs: Vec::new() });
        self.lists.sort_by_key(|l| l.name.to_lowercase());
        let at = self.position(&name)?;
        self.save(at);
        Some(at)
    }

    /// The first free "Playlist", "Playlist 2", ... name.
    pub fn fresh_name(&self) -> String {
        (1..).map(|n| if n == 1 { "Playlist".to_string() } else { format!("Playlist {n}") }).find(|n| self.position(n).is_none()).unwrap_or_default()
    }

    /// Renames playlist `i`; false when the name is unusable or taken by another.
    pub fn rename(&mut self, i: usize, name: &str) -> bool {
        let Some(name) = clean_name(name) else { return false };
        if i >= self.lists.len() || self.position(&name).is_some_and(|p| p != i) {
            return false;
        }
        let old = self.file(&self.lists[i].name.clone());
        let _ = std::fs::remove_file(old);
        self.lists[i].name = name.clone();
        self.save(i);
        self.lists.sort_by_key(|l| l.name.to_lowercase());
        true
    }

    pub fn delete(&mut self, i: usize) {
        if i < self.lists.len() {
            let l = self.lists.remove(i);
            let _ = std::fs::remove_file(self.file(&l.name));
        }
    }

    /// Appends a song (a song already in the list is not added twice).
    pub fn add(&mut self, i: usize, song: &Path) -> bool {
        match self.lists.get_mut(i) {
            Some(l) if !l.songs.iter().any(|p| p == song) => {
                l.songs.push(song.to_path_buf());
                self.save(i);
                true
            }
            _ => false,
        }
    }

    /// Takes the `at`-th song out of playlist `i`.
    pub fn remove(&mut self, i: usize, at: usize) -> bool {
        match self.lists.get_mut(i) {
            Some(l) if at < l.songs.len() => {
                l.songs.remove(at);
                self.save(i);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-playlists").join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn names_are_cleaned_for_the_file_system() {
        assert_eq!(clean_name("  Road/Trip: 1 "), Some("RoadTrip 1".into()));
        assert_eq!(clean_name("..hidden"), Some("hidden".into()));
        assert_eq!(clean_name("///"), None);
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), 60);
    }

    #[test]
    fn creates_adds_removes_and_survives_a_reopen() {
        let d = dir("basic");
        let mut p = Playlists::open(d.clone());
        assert!(p.lists().is_empty());
        let road = p.create("Road").unwrap();
        assert!(p.add(road, Path::new("/m/a.mp3")));
        assert!(p.add(road, Path::new("/m/b.mp3")));
        assert!(!p.add(road, Path::new("/m/a.mp3")), "no duplicates");
        assert!(p.create("road").is_none(), "names are unique, case aside");
        assert!(p.create("").is_none());
        assert!(p.remove(road, 0));
        assert!(!p.remove(road, 5));
        let text = std::fs::read_to_string(d.join("Road.m3u")).unwrap();
        assert_eq!(text, "#EXTM3U\n/m/b.mp3\n");
        let back = Playlists::open(d.clone());
        assert_eq!(back.lists(), [Playlist { name: "Road".into(), songs: vec!["/m/b.mp3".into()] }]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lists_stay_in_name_order_and_the_index_follows_the_name() {
        let d = dir("order");
        let mut p = Playlists::open(d.clone());
        p.create("Zen").unwrap();
        let a = p.create("alpha").unwrap();
        assert_eq!(a, 0);
        assert_eq!(p.lists()[1].name, "Zen");
        assert_eq!(p.fresh_name(), "Playlist");
        p.create("Playlist").unwrap();
        assert_eq!(p.fresh_name(), "Playlist 2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rename_moves_the_file_and_delete_removes_it() {
        let d = dir("rename");
        let mut p = Playlists::open(d.clone());
        let a = p.create("One").unwrap();
        p.add(a, Path::new("/m/a.mp3"));
        p.create("Two").unwrap();
        assert!(!p.rename(0, "Two"), "taken");
        assert!(!p.rename(0, "/"), "unusable");
        assert!(p.rename(0, "Uno"));
        assert!(!d.join("One.m3u").exists() && d.join("Uno.m3u").exists());
        assert_eq!(Playlists::open(d.clone()).lists().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Two", "Uno"]);
        let i = p.lists().iter().position(|l| l.name == "Uno").unwrap();
        p.delete(i);
        assert!(!d.join("Uno.m3u").exists());
        assert_eq!(p.lists().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
