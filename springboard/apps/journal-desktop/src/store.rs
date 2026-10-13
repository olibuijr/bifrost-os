//! The journal's files in the app's data directory: `journal.json` for the entries and `photos/` for the copies of
//! the photos. Every save writes a temporary file and renames it over the old one, so a crash mid-write keeps the
//! previous journal.

use crate::model::{self, Journal};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "journal.json";
pub const PHOTO_DIR: &str = "photos";

pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub fn photos_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(PHOTO_DIR)
}

/// Where a photo file of the journal lives. The name must be a photo file name (see `model::is_photo_name`).
pub fn photo_path(data_dir: &Path, name: &str) -> PathBuf {
    photos_dir(data_dir).join(name)
}

/// The saved journal, or an empty one when there is no file yet. A file that cannot be read or decoded is kept
/// aside as `journal.json.bad` and the journal starts empty, so the user's entries are never silently overwritten.
pub fn load(data_dir: &Path) -> Journal {
    let path = path_in(data_dir);
    let Ok(text) = fs::read_to_string(&path) else { return Journal::default() };
    match model::decode(&text) {
        Ok(journal) => journal,
        Err(_) => {
            let _ = fs::rename(&path, data_dir.join(format!("{FILE_NAME}.bad")));
            Journal::default()
        }
    }
}

/// Writes the journal atomically: temporary file, flush, rename.
pub fn save(data_dir: &Path, journal: &Journal) -> io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let path = path_in(data_dir);
    let tmp = data_dir.join(format!("{FILE_NAME}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(model::encode(journal).as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)
}

/// Copies an image file into the photo directory under `name`. Returns the number of bytes copied.
pub fn import_photo(data_dir: &Path, source: &Path, name: &str) -> io::Result<u64> {
    if !model::is_photo_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a photo file name"));
    }
    if !source.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "not a file"));
    }
    let dir = photos_dir(data_dir);
    fs::create_dir_all(&dir)?;
    fs::copy(source, dir.join(name))
}

/// Removes a photo file. A file that is already gone is not an error.
pub fn delete_photo(data_dir: &Path, name: &str) -> io::Result<()> {
    if !model::is_photo_name(name) {
        return Ok(());
    }
    match fs::remove_file(photo_path(data_dir, name)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Day;

    /// A scratch directory inside the crate's target folder (git-ignored), so tests never write to /tmp.
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("journal-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_then_load_restores_the_journal() {
        let dir = scratch("roundtrip");
        let mut j = Journal::default();
        let id = j.new_entry(Day::parse("2026-10-13").unwrap(), 100);
        j.set_text(id, "kept", 101);
        save(&dir, &j).unwrap();
        assert!(!dir.join("journal.json.tmp").exists());
        assert_eq!(load(&dir).entries, j.entries);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_loads_an_empty_journal() {
        let dir = scratch("missing");
        assert_eq!(load(&dir), Journal::default());
    }

    #[test]
    fn a_broken_file_is_moved_aside() {
        let dir = scratch("broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(path_in(&dir), "not json").unwrap();
        assert!(load(&dir).entries.is_empty());
        assert!(dir.join("journal.json.bad").exists());
        assert!(!path_in(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn photos_are_copied_and_deleted() {
        let dir = scratch("photos");
        let src_dir = scratch("photos-source");
        fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("beach.png");
        fs::write(&src, b"fake image bytes").unwrap();
        assert_eq!(import_photo(&dir, &src, "3-1.png").unwrap(), 16);
        assert!(photo_path(&dir, "3-1.png").exists());
        assert!(import_photo(&dir, &src, "../escape.png").is_err());
        assert!(import_photo(&dir, &src_dir.join("missing.png"), "3-2.png").is_err());
        delete_photo(&dir, "3-1.png").unwrap();
        assert!(!photo_path(&dir, "3-1.png").exists());
        delete_photo(&dir, "3-1.png").unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src_dir);
    }
}
