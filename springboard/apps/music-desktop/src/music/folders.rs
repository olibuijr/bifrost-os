//! Playlist folders: named groups of playlists, shown nested under their folder in the Playlists tab. The folders and
//! each playlist's folder live in `folders.txt` beside the playlist files; a playlist is kept in a folder by its name,
//! as the playlist files are kept by name. Pure file logic and the tab's row order, no drawing.

use super::playlists::{clean_name, Playlist};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The folders, in name order, and which folder each playlist is in (by lower-case playlist name).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folders {
    file: Option<PathBuf>,
    names: Vec<String>,
    members: BTreeMap<String, String>,
}

/// A row of the Playlists tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    NewPlaylist,
    NewSmart,
    NewFolder,
    /// folder `i` (in `Folders::names` order), with its playlists under it unless it is collapsed
    Folder(usize),
    /// playlist `index` (in the playlists' order); indented when it is in a folder
    Playlist { index: usize, in_folder: bool },
    /// smart playlist `i`
    Smart(usize),
}

fn key(playlist: &str) -> String {
    playlist.to_lowercase()
}

impl Folders {
    /// Reads the folders kept in `file`, which may not exist yet. A line that does not parse is skipped.
    pub fn open(file: PathBuf) -> Self {
        let mut folders = Folders { file: Some(file.clone()), ..Folders::default() };
        if let Ok(text) = std::fs::read_to_string(&file) {
            for line in text.lines() {
                let cols: Vec<&str> = line.split('\t').collect();
                match cols.as_slice() {
                    ["F", name] if !name.is_empty() => folders.names.push(name.to_string()),
                    ["M", playlist, folder] if !playlist.is_empty() && !folder.is_empty() => {
                        folders.members.insert(key(playlist), folder.to_string());
                    }
                    _ => {}
                }
            }
        }
        folders.names.sort_by_key(|n| n.to_lowercase());
        folders.names.dedup();
        folders
    }

    /// The folder names, in name order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.to_lowercase() == name.to_lowercase())
    }

    /// The first free "Folder", "Folder 2", ... name.
    pub fn fresh_name(&self) -> String {
        (1..).map(|n| if n == 1 { "Folder".to_string() } else { format!("Folder {n}") }).find(|n| self.position(n).is_none()).unwrap_or_default()
    }

    /// Makes a folder; its index, or None when the name is unusable or taken.
    pub fn create(&mut self, name: &str) -> Option<usize> {
        let name = clean_name(name)?;
        if self.position(&name).is_some() {
            return None;
        }
        self.names.push(name.clone());
        self.names.sort_by_key(|n| n.to_lowercase());
        self.save();
        self.position(&name)
    }

    /// The name of the folder `playlist` is in, if it is in one that exists.
    pub fn folder_of(&self, playlist: &str) -> Option<&str> {
        let name = self.members.get(&key(playlist))?;
        self.names.iter().find(|n| *n == name).map(String::as_str)
    }

    /// The index of the folder `playlist` is in, if it is in one.
    pub fn folder_index(&self, playlist: &str) -> Option<usize> {
        let name = self.folder_of(playlist)?;
        self.names.iter().position(|n| n == name)
    }

    /// Renames folder `index` to `to`; its playlists stay in it. Returns the folder's index in name order after the
    /// rename, or None when there is no such folder, the name is unusable or another folder already has it.
    pub fn rename(&mut self, index: usize, to: &str) -> Option<usize> {
        let from = self.names.get(index)?.clone();
        let to = clean_name(to)?;
        if self.position(&to).is_some_and(|other| other != index) {
            return None;
        }
        for folder in self.members.values_mut() {
            if *folder == from {
                *folder = to.clone();
            }
        }
        self.names[index] = to.clone();
        self.names.sort_by_key(|n| n.to_lowercase());
        self.save();
        self.position(&to)
    }

    /// Deletes folder `index`. Its playlists are kept and listed in no folder. False when there is no such folder.
    pub fn delete(&mut self, index: usize) -> bool {
        if index >= self.names.len() {
            return false;
        }
        let name = self.names.remove(index);
        self.members.retain(|_, folder| *folder != name);
        self.save();
        true
    }

    /// Puts `playlist` in folder `folder` (an index into `names`), or takes it out of its folder for None. True when
    /// its folder changed.
    pub fn put(&mut self, playlist: &str, folder: Option<usize>) -> bool {
        let name = folder.and_then(|i| self.names.get(i)).cloned();
        let before = self.members.get(&key(playlist)).cloned();
        match name {
            Some(name) => {
                self.members.insert(key(playlist), name);
            }
            None => {
                self.members.remove(&key(playlist));
            }
        }
        let changed = self.members.get(&key(playlist)).cloned() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// A playlist was renamed: it stays in its folder under the new name.
    pub fn rename_playlist(&mut self, from: &str, to: &str) {
        if let Some(folder) = self.members.remove(&key(from)) {
            self.members.insert(key(to), folder);
            self.save();
        }
    }

    /// A playlist was deleted: it leaves its folder.
    pub fn forget_playlist(&mut self, playlist: &str) {
        if self.members.remove(&key(playlist)).is_some() {
            self.save();
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = String::new();
        for name in &self.names {
            text.push_str(&format!("F\t{name}\n"));
        }
        for (playlist, folder) in &self.members {
            text.push_str(&format!("M\t{playlist}\t{folder}\n"));
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

/// The rows of the Playlists tab: the three New rows; then each folder, followed by its playlists unless the folder
/// is in `collapsed`; then the playlists in no folder; then the smart playlists (`smart` of them).
pub fn rows(folders: &Folders, lists: &[Playlist], smart: usize, collapsed: &[String]) -> Vec<Row> {
    let mut rows = vec![Row::NewPlaylist, Row::NewSmart, Row::NewFolder];
    for (f, name) in folders.names().iter().enumerate() {
        rows.push(Row::Folder(f));
        if collapsed.contains(name) {
            continue;
        }
        rows.extend(lists.iter().enumerate().filter(|(_, l)| folders.folder_index(&l.name) == Some(f)).map(|(index, _)| Row::Playlist { index, in_folder: true }));
    }
    rows.extend(lists.iter().enumerate().filter(|(_, l)| folders.folder_index(&l.name).is_none()).map(|(index, _)| Row::Playlist { index, in_folder: false }));
    rows.extend((0..smart).map(Row::Smart));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn list(name: &str) -> Playlist {
        Playlist { name: name.into(), songs: Vec::new() }
    }

    #[test]
    fn a_folder_is_made_once_and_named_freshly() {
        let mut f = Folders::default();
        assert_eq!(f.fresh_name(), "Folder");
        assert_eq!(f.create("Road"), Some(0));
        assert!(f.create("road").is_none(), "names are unique, case aside");
        assert!(f.create("///").is_none());
        assert_eq!(f.fresh_name(), "Folder");
        f.create("Folder").unwrap();
        assert_eq!(f.fresh_name(), "Folder 2");
        assert_eq!(f.names(), ["Folder", "Road"]);
    }

    #[test]
    fn a_playlist_goes_into_a_folder_and_back_out() {
        let mut f = Folders::default();
        f.create("Road").unwrap();
        assert!(f.put("Sunday Mix", Some(0)));
        assert_eq!(f.folder_of("sunday mix"), Some("Road"), "the playlist is found whatever its case");
        assert!(!f.put("Sunday Mix", Some(0)), "already there");
        assert!(f.put("Sunday Mix", None));
        assert_eq!(f.folder_of("Sunday Mix"), None);
        assert!(!f.put("Sunday Mix", Some(9)), "no such folder leaves it where it is");
    }

    #[test]
    fn renames_and_deletes_of_playlists_follow_into_the_folders() {
        let mut f = Folders::default();
        f.create("Road").unwrap();
        f.put("One", Some(0));
        f.rename_playlist("One", "Uno");
        assert_eq!(f.folder_of("Uno"), Some("Road"));
        assert_eq!(f.folder_of("One"), None);
        f.forget_playlist("Uno");
        assert_eq!(f.folder_of("Uno"), None);
    }

    #[test]
    fn the_folders_survive_a_reopen() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-folders/reopen");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("folders.txt");
        let mut f = Folders::open(file.clone());
        f.create("Road").unwrap();
        f.create("Gym").unwrap();
        f.put("Sunday Mix", Some(1));
        let back = Folders::open(file);
        assert_eq!(back.names(), ["Gym", "Road"]);
        assert_eq!(back.folder_of("sunday mix"), Some("Road"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tab_lists_folders_with_their_playlists_and_the_rest_after_them() {
        let mut f = Folders::default();
        f.create("Road").unwrap();
        f.create("Gym").unwrap();
        f.put("Two", Some(0));
        f.put("Three", Some(1));
        let lists = [list("One"), list("Two"), list("Three")];
        let rows = rows(&f, &lists, 1, &[]);
        assert_eq!(
            rows,
            [
                Row::NewPlaylist,
                Row::NewSmart,
                Row::NewFolder,
                Row::Folder(0),
                Row::Playlist { index: 1, in_folder: true },
                Row::Folder(1),
                Row::Playlist { index: 2, in_folder: true },
                Row::Playlist { index: 0, in_folder: false },
                Row::Smart(0),
            ]
        );
    }

    #[test]
    fn a_collapsed_folder_hides_its_playlists_only() {
        let mut f = Folders::default();
        f.create("Road").unwrap();
        f.put("Two", Some(0));
        let lists = [list("Two")];
        let rows = rows(&f, &lists, 0, &["Road".to_string()]);
        assert_eq!(rows[3..], [Row::Folder(0)]);
        assert_eq!(rows.len(), 4, "no playlist is listed twice or lost");
    }

    #[test]
    fn a_membership_of_a_missing_folder_lists_the_playlist_at_the_top() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-folders/ghost");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("folders.txt"), "M\tOne\tGhost\n").unwrap();
        let f = Folders::open(dir.join("folders.txt"));
        assert_eq!(f.folder_of("One"), None, "the folder Ghost does not exist");
        let lists = [list("One")];
        assert_eq!(rows(&f, &lists, 0, &[])[3..], [Row::Playlist { index: 0, in_folder: false }]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
