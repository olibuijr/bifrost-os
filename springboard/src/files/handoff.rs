//! Show in Files: a request to open the Files app at a folder, and perhaps select one item in it. The shell (or the
//! `bifrost-apps --open` helper) writes the folder into the Files app's data folder as `open-request`; the running app
//! takes the file and navigates there. A file channel keeps the app protocol unchanged.
//!
//! The item to select travels beside it as `open-select` (the item's name, one line), written before the folder. A
//! Files app that does not know the second file still opens the folder; a folder request without an item removes it.

use std::io;
use std::path::{Path, PathBuf};

/// The request file name inside an app's data folder.
pub const REQUEST: &str = "open-request";
/// The file beside `REQUEST` that names the item to select in the folder.
pub const SELECT: &str = "open-select";

/// A request taken from the data folder: the folder to show and, for "show this file", the item to select in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub folder: PathBuf,
    /// The item's full path: always directly inside `folder`.
    pub select: Option<PathBuf>,
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why)
}

/// Whether `name` is one path component that can be written on a line: not empty, not `.` or `..`, with no `/`, line
/// break or NUL.
fn is_item_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\n', '\0'])
}

/// Writes `text` and a newline as `name` in `data_dir`, replacing any earlier file in one step.
fn write_line(data_dir: &Path, name: &str, text: &str) -> io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let tmp = data_dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, format!("{text}\n"))?;
    std::fs::rename(&tmp, data_dir.join(name))
}

/// The folder as request text: an absolute path to an existing directory, with no line break or NUL.
fn folder_text(folder: &Path) -> io::Result<&str> {
    let text = folder.to_str().filter(|s| s.starts_with('/') && !s.contains('\n') && !s.contains('\0'));
    let Some(text) = text else {
        return Err(invalid("Expected an absolute folder path"));
    };
    if !folder.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Folder not found"));
    }
    Ok(text)
}

/// Writes `folder` as the pending request in `data_dir` (replacing any earlier one, and dropping an earlier item to
/// select). The folder must be an absolute path to an existing directory.
pub fn write_request(data_dir: &Path, folder: &Path) -> io::Result<()> {
    let text = folder_text(folder)?;
    let _ = std::fs::remove_file(data_dir.join(SELECT));
    write_line(data_dir, REQUEST, text)
}

/// Writes a request to show `item` in the folder that holds it, selected. The item must be an absolute path to an
/// existing file or folder (a link counts, whatever it points at) that has a folder above it.
pub fn write_reveal(data_dir: &Path, item: &Path) -> io::Result<()> {
    if !item.is_absolute() {
        return Err(invalid("Expected an absolute file path"));
    }
    let name = item.file_name().and_then(|n| n.to_str()).filter(|n| is_item_name(n));
    let (Some(name), Some(folder)) = (name, item.parent()) else {
        return Err(invalid("Expected a file inside a folder"));
    };
    let text = folder_text(folder)?;
    if item.symlink_metadata().is_err() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "File not found"));
    }
    // the item first: the app looks for it only once the folder request is there
    write_line(data_dir, SELECT, name)?;
    write_line(data_dir, REQUEST, text)
}

/// Removes a pending request from `data_dir` (an app that could not be opened never reads it).
pub fn clear_request(data_dir: &Path) {
    let _ = std::fs::remove_file(data_dir.join(REQUEST));
    let _ = std::fs::remove_file(data_dir.join(SELECT));
}

/// Takes the pending request from `data_dir`: the folder if it still exists, else None, and the item to select when
/// one was named. The files are removed either way, so a bad request is not retried.
pub fn take_request(data_dir: &Path) -> Option<Request> {
    let text = std::fs::read_to_string(data_dir.join(REQUEST)).ok()?;
    let select = std::fs::read_to_string(data_dir.join(SELECT)).ok();
    clear_request(data_dir);
    let folder = PathBuf::from(text.trim_end_matches('\n'));
    if !(folder.is_absolute() && folder.is_dir()) {
        return None;
    }
    let select = select.as_deref().map(|s| s.strip_suffix('\n').unwrap_or(s)).filter(|n| is_item_name(n)).map(|n| folder.join(n));
    Some(Request { folder, select })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("handoff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_request_is_written_taken_once_and_removed() {
        let dir = temp("take");
        let data = dir.join("data");
        let folder = dir.join("My Documents");
        std::fs::create_dir_all(&folder).unwrap();
        write_request(&data, &folder).unwrap();
        assert_eq!(take_request(&data), Some(Request { folder: folder.clone(), select: None }), "a folder with a space survives");
        assert_eq!(take_request(&data), None, "taken once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_requests_are_refused_or_dropped() {
        let dir = temp("bad");
        let data = dir.join("data");
        assert!(write_request(&data, Path::new("relative/path")).is_err());
        assert!(write_request(&data, &dir.join("missing")).is_err());
        // a request whose folder has gone is dropped, not returned
        let gone = dir.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        write_request(&data, &gone).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert_eq!(take_request(&data), None);
        assert!(!data.join(REQUEST).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reveal_request_carries_the_folder_and_the_item_to_select() {
        let dir = temp("reveal");
        let data = dir.join("data");
        let folder = dir.join("My Documents");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("Q1 report.pdf");
        std::fs::write(&file, b"x").unwrap();
        write_reveal(&data, &file).unwrap();
        assert_eq!(std::fs::read_to_string(data.join(REQUEST)).unwrap(), format!("{}\n", folder.display()), "the folder file is what an older app reads");
        assert_eq!(std::fs::read_to_string(data.join(SELECT)).unwrap(), "Q1 report.pdf\n");
        assert_eq!(take_request(&data), Some(Request { folder: folder.clone(), select: Some(file.clone()) }));
        assert_eq!(take_request(&data), None, "taken once");
        assert!(!data.join(SELECT).exists() && !data.join(REQUEST).exists());
        // a folder inside the folder is selected the same way
        let sub = folder.join("Sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_reveal(&data, &sub).unwrap();
        assert_eq!(take_request(&data), Some(Request { folder: folder.clone(), select: Some(sub) }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_request_drops_an_item_left_by_an_earlier_reveal() {
        let dir = temp("stale");
        let data = dir.join("data");
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        // an app that never read the item (an older one reads the folder only)
        write_reveal(&data, &file).unwrap();
        std::fs::remove_file(data.join(REQUEST)).unwrap();
        assert_eq!(take_request(&data), None, "an item with no folder request is not a request");
        write_reveal(&data, &file).unwrap();
        write_request(&data, &dir).unwrap();
        assert_eq!(take_request(&data), Some(Request { folder: dir.clone(), select: None }));
        // a cleared request leaves nothing behind
        write_reveal(&data, &file).unwrap();
        clear_request(&data);
        assert_eq!(take_request(&data), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_reveal_requests_are_refused_and_bad_item_names_are_ignored() {
        let dir = temp("badreveal");
        let data = dir.join("data");
        assert!(write_reveal(&data, Path::new("relative/a.txt")).is_err());
        assert!(write_reveal(&data, &dir.join("missing.txt")).is_err());
        assert!(write_reveal(&data, &dir.join("missing/a.txt")).is_err(), "no such folder");
        assert!(write_reveal(&data, Path::new("/")).is_err(), "the root has no folder above it");
        assert!(write_reveal(&data, &dir.join("..")).is_err());
        assert!(!data.join(REQUEST).exists() && !data.join(SELECT).exists(), "a refused request writes nothing");
        // an item name that is not one path component never leaves the folder
        for name in ["../escape", "a/b", "..", ".", "", "two\nlines"] {
            write_request(&data, &dir).unwrap();
            std::fs::write(data.join(SELECT), format!("{name}\n")).unwrap();
            assert_eq!(take_request(&data), Some(Request { folder: dir.clone(), select: None }), "{name:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
