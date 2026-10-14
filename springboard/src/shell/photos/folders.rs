//! Pure album folder logic: a folder groups user albums under one name, and an album sits in at most one folder.
//! Folders are kept in the photos preferences as `uf.<n>.name` (the name) and `uf.<n>.albums` (the user album
//! slots, comma separated). The picker reads them back with [`decode`] and writes them with [`writes`].

use std::fmt;

/// Folders the library can hold.
pub const MAX_FOLDERS: usize = 8;
/// Longest folder name kept, in characters.
pub const MAX_NAME: usize = 60;

/// One folder and the user album slots it groups, in the order they were added.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AlbumFolder {
    pub name: String,
    pub albums: Vec<usize>,
}

/// Why a folder could not be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderError {
    /// The name is blank once control characters and spaces are removed.
    Empty,
    /// A folder with the same name, ignoring case, exists.
    Duplicate,
    /// The library already holds [`MAX_FOLDERS`] folders.
    Full,
}

impl fmt::Display for FolderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FolderError::Empty => "a folder needs a name",
            FolderError::Duplicate => "a folder with that name exists",
            FolderError::Full => "the library holds no more folders",
        })
    }
}

/// The name as kept: control characters dropped, surrounding spaces trimmed, cut to [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> String {
    let kept: String = name.chars().filter(|c| !c.is_control()).collect();
    kept.trim().chars().take(MAX_NAME).collect()
}

/// Adds an empty folder named `name` and returns its index.
pub fn new_folder(folders: &mut Vec<AlbumFolder>, name: &str) -> Result<usize, FolderError> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err(FolderError::Empty);
    }
    if folders.len() >= MAX_FOLDERS {
        return Err(FolderError::Full);
    }
    if folders.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
        return Err(FolderError::Duplicate);
    }
    folders.push(AlbumFolder { name, albums: Vec::new() });
    Ok(folders.len() - 1)
}

/// Puts user album `slot` into folder `folder`. The album leaves any other folder first, so it sits in one folder
/// at most. Returns false when there is no such folder.
pub fn move_album(folders: &mut [AlbumFolder], folder: usize, slot: usize) -> bool {
    if folder >= folders.len() {
        return false;
    }
    for f in folders.iter_mut() {
        f.albums.retain(|&s| s != slot);
    }
    folders[folder].albums.push(slot);
    true
}

/// The folder that holds user album `slot`, if any.
pub fn folder_of(folders: &[AlbumFolder], slot: usize) -> Option<usize> {
    folders.iter().position(|f| f.albums.contains(&slot))
}

/// Keeps folder membership in step with the album list when user album `slot` is deleted: the album leaves its
/// folder and every later slot moves down one, as the album slots do.
pub fn album_deleted(folders: &mut [AlbumFolder], slot: usize) {
    for f in folders.iter_mut() {
        f.albums.retain(|&s| s != slot);
        for s in f.albums.iter_mut().filter(|s| **s > slot) {
            *s -= 1;
        }
    }
}

/// Removes folder `folder`. Its albums stay in the album list, outside any folder.
pub fn delete_folder(folders: &mut Vec<AlbumFolder>, folder: usize) -> bool {
    if folder >= folders.len() {
        return false;
    }
    folders.remove(folder);
    true
}

/// Reads the folders stored under `uf.<n>.*`, stopping at the first slot without a name. `get` looks a key up in
/// the preferences. Album slots that do not parse are skipped.
pub fn decode<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Vec<AlbumFolder> {
    let mut folders = Vec::new();
    for n in 0..MAX_FOLDERS {
        let Some(name) = get(&format!("uf.{n}.name")).filter(|s| !s.is_empty()) else { break };
        let albums = get(&format!("uf.{n}.albums"))
            .unwrap_or("")
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .collect();
        folders.push(AlbumFolder { name: name.to_string(), albums });
    }
    folders
}

/// The preference writes that store `folders`, clearing the slots of the `previous` folders that no longer exist,
/// so the stored slots stay contiguous from zero.
pub fn writes(folders: &[AlbumFolder], previous: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for n in 0..previous.max(folders.len()) {
        match folders.get(n) {
            Some(f) => {
                out.push((format!("uf.{n}.name"), f.name.clone()));
                let albums: Vec<String> = f.albums.iter().map(|s| s.to_string()).collect();
                out.push((format!("uf.{n}.albums"), albums.join(",")));
            }
            None => {
                out.push((format!("uf.{n}.name"), String::new()));
                out.push((format!("uf.{n}.albums"), String::new()));
            }
        }
    }
    out
}
