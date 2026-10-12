//! File coordination for the Files app's writes. An item is changed under the shared file coordination lock on its
//! folder (SDK `file_coordination`), so another coordinating app never reads a half-moved item. A lock that stays
//! busy past the timeout is reported to the user as "in use by another app" and the item is left alone.
use crate::*;
use bifrost_app::file_coordination::{CoordinationError, NativeFileCoordinator};
use std::path::{Path, PathBuf};

/// Runs `f` (a change to the item at `path`) while holding the item's coordination lock for writing.
pub(crate) fn writing<R>(path: &Path, f: impl FnOnce() -> R) -> Result<R, CoordinationError> {
    NativeFileCoordinator.coordinate_writing(path, |_| f())
}

/// The name of the placeholder whose parent is the folder being locked: the lock is taken on the folder itself,
/// because a batch writes several items into it (a copy, an archive, an extracted folder).
const FOLDER_LOCK: &str = ".files-coordinate";

/// The path whose parent is `dir`: `coordinate_writing` locks the parent folder of the path it is given.
pub(crate) fn folder_lock_path(dir: &Path) -> PathBuf {
    dir.join(FOLDER_LOCK)
}

/// Runs `f` (a batch of changes inside the folder `dir`) while holding the folder's coordination lock for writing.
/// A reader in another coordinating app holds the same folder shared while it reads, so a file open there is not
/// replaced under it.
pub(crate) fn writing_in<R>(dir: &Path, f: impl FnOnce() -> R) -> Result<R, CoordinationError> {
    writing(&folder_lock_path(dir), f)
}

/// The report for a batch that could not take its folder's lock: every item fails with the coordination sentence.
pub(crate) fn refused(sources: &[PathBuf], e: &CoordinationError) -> Report {
    let why = message(e);
    let mut report = Report::default();
    report.failed = sources.iter().map(|p| (p.clone(), why.clone())).collect();
    report
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
    fn the_folder_lock_sits_inside_the_folder() {
        let dir = PathBuf::from("/srv/share");
        assert_eq!(folder_lock_path(&dir).parent(), Some(dir.as_path()));
    }

    #[test]
    fn a_refused_batch_fails_every_item_with_the_sentence() {
        let src = vec![PathBuf::from("/a/one.txt"), PathBuf::from("/a/two.txt")];
        let report = refused(&src, &CoordinationError::Timeout);
        assert!(report.done.is_empty());
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[1], (PathBuf::from("/a/two.txt"), "The item is in use by another app.".to_string()));
    }

    #[test]
    fn a_batch_in_a_folder_runs_under_the_folder_lock() {
        let dir = scratch("batch");
        let out = writing_in(&dir, || 7);
        assert_eq!(out.ok(), Some(7));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_timeout_has_the_user_facing_sentence() {
        assert_eq!(message(&CoordinationError::Timeout), "The item is in use by another app.");
    }
}
