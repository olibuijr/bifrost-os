//! File coordination for the Files app's writes. An item is changed under the shared file coordination lock on its
//! folder (SDK `file_coordination`), so another coordinating app never reads a half-moved item. A lock that stays
//! busy past the timeout is reported to the user as "in use by another app" and the item is left alone.
use bifrost_app::file_coordination::{CoordinationError, NativeFileCoordinator};
use std::path::Path;

/// Runs `f` (a change to the item at `path`) while holding the item's coordination lock for writing.
pub(crate) fn writing<R>(path: &Path, f: impl FnOnce() -> R) -> Result<R, CoordinationError> {
    NativeFileCoordinator.coordinate_writing(path, |_| f())
}

/// The sentence the report shows for a coordination failure.
pub(crate) fn message(e: &CoordinationError) -> String {
    match e {
        CoordinationError::Timeout => "The item is in use by another app.".to_string(),
        CoordinationError::Io(e) => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-coordinate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_coordinated_change_runs_and_returns_its_result() {
        let dir = scratch("run");
        let item = dir.join("note.txt");
        std::fs::write(&item, "hello").unwrap();
        let out = writing(&item, || std::fs::remove_file(&item).is_ok());
        assert_eq!(out.ok(), Some(true));
        assert!(!item.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_reports_the_io_error_not_a_timeout() {
        let item = std::env::temp_dir().join("files-coordinate-missing-no-such-dir").join("x");
        let err = writing(&item, || ()).err().expect("the folder does not exist");
        assert!(matches!(err, CoordinationError::Io(_)));
        assert_ne!(message(&err), "The item is in use by another app.");
    }

    #[test]
    fn a_timeout_has_the_user_facing_sentence() {
        assert_eq!(message(&CoordinationError::Timeout), "The item is in use by another app.");
    }
}
