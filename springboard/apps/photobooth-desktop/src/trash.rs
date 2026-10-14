//! Photo Booth's Trash. A deleted capture moves into the desktop Trash folder, laid out as the common desktop trash
//! specification describes it: `files/` holds the picture and `info/` holds a record of where it came from and when it
//! was deleted. The file manager shows that folder, and Undo Delete moves the picture back out of it.

use springboard::shell::photos::library::Date;
use std::io;
use std::path::{Path, PathBuf};

/// Where a deleted capture sits in the Trash: the picture in `files/` and its record in `info/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub place: PathBuf,
    pub record: PathBuf,
}

/// The Trash folder: `Trash` inside the data folder when that is an absolute path, else inside `home/.local/share`.
pub fn trash_dir(home: &Path, data_home: Option<PathBuf>) -> PathBuf {
    data_home.filter(|dir| dir.is_absolute()).unwrap_or_else(|| home.join(".local").join("share")).join("Trash")
}

/// The record of a file that came from `original` and was deleted at `deleted`, in the specification's format.
pub fn info_text(original: &Path, deleted: &Date) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
        percent_encode(original),
        deleted.year,
        deleted.month,
        deleted.day,
        deleted.hour,
        deleted.min,
        deleted.sec
    )
}

/// The path with every byte outside the unreserved characters and `/` written as `%XX`, as the record's Path field needs.
pub fn percent_encode(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(char::from(*byte)),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Moves the file at `file` into the Trash under `trash` and writes its record. The picture keeps its name unless the
/// Trash already holds one like it. When the file cannot be moved, nothing is left behind and the error says why.
pub fn put(trash: &Path, file: &Path, deleted: &Date) -> io::Result<Trashed> {
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = free_name_by(|candidate| taken_in(trash, candidate), &name_of(file)?);
    let place = files.join(&name);
    let record = info.join(format!("{name}.trashinfo"));
    std::fs::write(&record, info_text(file, deleted))?;
    if let Err(e) = move_file(file, &place) {
        let _ = std::fs::remove_file(&record);
        return Err(e);
    }
    Ok(Trashed { place, record })
}

/// Moves a trashed picture back to `original` and removes its record. When a file has taken the original's place since,
/// the picture is written beside it under a free name instead. Returns the path the picture was written to.
pub fn restore(trashed: &Trashed, original: &Path) -> io::Result<PathBuf> {
    let target = if original.exists() {
        let dir = original.parent().map(Path::to_path_buf).unwrap_or_default();
        let free = free_name_by(|candidate| dir.join(candidate).exists(), &name_of(original)?);
        dir.join(free)
    } else {
        original.to_path_buf()
    };
    move_file(&trashed.place, &target)?;
    let _ = std::fs::remove_file(&trashed.record);
    Ok(target)
}

/// `name` when `taken` does not report it, else `stem 2.ext`, `stem 3.ext`... with the extension kept.
fn free_name_by(taken: impl Fn(&str) -> bool, name: &str) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    let mut n = 2;
    loop {
        let candidate = format!("{stem} {n}{ext}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// True when the Trash already holds a picture or a record called `name`.
fn taken_in(trash: &Path, name: &str) -> bool {
    trash.join("files").join(name).exists() || trash.join("info").join(format!("{name}.trashinfo")).exists()
}

fn name_of(path: &Path) -> io::Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))
}

/// Moves a file, copying it and removing the original when a plain rename is not possible across file systems.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn date() -> Date {
        Date { year: 2026, month: 10, day: 13, hour: 14, min: 5, sec: 9, wday: 2 }
    }

    #[test]
    fn the_trash_folder_follows_the_data_folder_when_that_is_absolute() {
        let home = Path::new("/home/x");
        assert_eq!(trash_dir(home, None), PathBuf::from("/home/x/.local/share/Trash"));
        assert_eq!(trash_dir(home, Some(PathBuf::from("relative"))), PathBuf::from("/home/x/.local/share/Trash"));
        assert_eq!(trash_dir(home, Some(PathBuf::from("/data"))), PathBuf::from("/data/Trash"));
    }

    #[test]
    fn the_record_has_the_specification_header_the_original_path_and_the_local_time() {
        let text = info_text(Path::new("/home/x/Pictures/Photo Booth Library/a.jpg"), &date());
        assert_eq!(text, "[Trash Info]\nPath=/home/x/Pictures/Photo%20Booth%20Library/a.jpg\nDeletionDate=2026-10-13T14:05:09\n");
    }

    #[test]
    fn paths_keep_their_slashes_and_encode_every_other_byte_outside_the_unreserved_set() {
        assert_eq!(percent_encode(Path::new("/a/b-c_d.e~f")), "/a/b-c_d.e~f");
        assert_eq!(percent_encode(Path::new("/a/100%\u{e4}.jpg")), "/a/100%25%C3%A4.jpg");
    }

    #[test]
    fn a_deleted_file_moves_into_the_trash_with_its_record() {
        let dir = Scratch::new("photobooth-trash-put");
        let original = dir.0.join("Pictures").join("a.jpg");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, b"picture").unwrap();
        let trash = dir.0.join("Trash");
        let trashed = put(&trash, &original, &date()).unwrap();
        assert!(!original.exists());
        assert_eq!(trashed.place, trash.join("files").join("a.jpg"));
        assert_eq!(std::fs::read(&trashed.place).unwrap(), b"picture");
        assert_eq!(trashed.record, trash.join("info").join("a.jpg.trashinfo"));
        assert_eq!(std::fs::read_to_string(&trashed.record).unwrap(), info_text(&original, &date()));
    }

    #[test]
    fn a_file_named_like_one_in_the_trash_gets_a_free_name() {
        let dir = Scratch::new("photobooth-trash-names");
        let trash = dir.0.join("Trash");
        let original = dir.0.join("a.jpg");
        let mut places = Vec::new();
        for _ in 0..3 {
            std::fs::write(&original, b"x").unwrap();
            places.push(put(&trash, &original, &date()).unwrap().place);
        }
        assert_eq!(places, vec![trash.join("files/a.jpg"), trash.join("files/a 2.jpg"), trash.join("files/a 3.jpg")]);
    }

    #[test]
    fn a_missing_file_fails_with_not_found_and_leaves_no_record() {
        let dir = Scratch::new("photobooth-trash-missing");
        let trash = dir.0.join("Trash");
        let err = put(&trash, &dir.0.join("gone.jpg"), &date()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(std::fs::read_dir(trash.join("info")).unwrap().next().is_none());
    }

    #[test]
    fn restore_puts_the_picture_back_and_removes_its_record() {
        let dir = Scratch::new("photobooth-trash-restore");
        let original = dir.0.join("a.jpg");
        std::fs::write(&original, b"picture").unwrap();
        let trashed = put(&dir.0.join("Trash"), &original, &date()).unwrap();
        let back = restore(&trashed, &original).unwrap();
        assert_eq!(back, original);
        assert_eq!(std::fs::read(&original).unwrap(), b"picture");
        assert!(!trashed.place.exists() && !trashed.record.exists());
    }

    #[test]
    fn a_picture_that_took_the_original_place_is_kept_and_the_restore_gets_a_free_name() {
        let dir = Scratch::new("photobooth-trash-clash");
        let original = dir.0.join("a.jpg");
        std::fs::write(&original, b"old").unwrap();
        let trashed = put(&dir.0.join("Trash"), &original, &date()).unwrap();
        std::fs::write(&original, b"new").unwrap();
        let back = restore(&trashed, &original).unwrap();
        assert_eq!(back, dir.0.join("a 2.jpg"));
        assert_eq!(std::fs::read(&original).unwrap(), b"new");
        assert_eq!(std::fs::read(&back).unwrap(), b"old");
    }

    #[test]
    fn free_names_keep_the_extension_and_a_name_without_one_gets_the_number_last() {
        assert_eq!(free_name_by(|candidate| candidate == "notes", "notes"), "notes 2");
        assert_eq!(free_name_by(|_| false, "a.jpg"), "a.jpg");
        assert_eq!(free_name_by(|candidate| candidate == "a.jpg" || candidate == "a 2.jpg", "a.jpg"), "a 3.jpg");
    }
}
