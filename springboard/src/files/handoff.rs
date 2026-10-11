//! Show in Files: a request to open the Files app at a folder. The shell (or the
//! `bifrost-apps --open` helper) writes the folder into the Files app's data folder
//! as `open-request`; the running app takes the file and navigates there. A file
//! channel keeps the app protocol unchanged.

use std::io;
use std::path::{Path, PathBuf};

/// The request file name inside an app's data folder.
pub const REQUEST: &str = "open-request";

/// Writes `folder` as the pending request in `data_dir` (replacing any earlier one).
/// The folder must be an absolute path to an existing directory.
pub fn write_request(data_dir: &Path, folder: &Path) -> io::Result<()> {
    let text = folder.to_str().filter(|s| s.starts_with('/') && !s.contains('\n') && !s.contains('\0'));
    let Some(text) = text else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Expected an absolute folder path"));
    };
    if !folder.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Folder not found"));
    }
    std::fs::create_dir_all(data_dir)?;
    let tmp = data_dir.join(format!("{REQUEST}.tmp"));
    std::fs::write(&tmp, format!("{text}\n"))?;
    std::fs::rename(&tmp, data_dir.join(REQUEST))
}

/// Takes the pending request from `data_dir`: the folder if it still exists, else None.
/// The file is removed either way, so a bad request is not retried.
pub fn take_request(data_dir: &Path) -> Option<PathBuf> {
    let path = data_dir.join(REQUEST);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let folder = PathBuf::from(text.trim_end_matches('\n'));
    (folder.is_absolute() && folder.is_dir()).then_some(folder)
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
        assert_eq!(take_request(&data), Some(folder.clone()), "a folder with a space survives");
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
}
