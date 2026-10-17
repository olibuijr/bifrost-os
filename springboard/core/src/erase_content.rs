//! Erase All Content and Settings: returns the shell's data directory to a first-start state while keeping the system.
//!
//! The running shell holds its data in memory and writes it back, so the erase is staged and done at the next start,
//! before anything loads (the way a staged backup restore is). Settings calls [`request`], which writes a marker file, and
//! restarts the shell; the binary calls [`apply_pending`] first thing. Nothing here runs while the shell is up.
//!
//! What goes: every entry of the data directory (settings, Notes, Contacts, Calendar, per-app containers under `appdata`,
//! local backups, the keychain, accounts, crash logs). The keychain goes first: its items are sealed under a master key
//! file, so deleting that key makes every copy of the items unreadable even where the file system keeps old blocks. What
//! stays: the update bookkeeping in [`KEEP`], which belongs to the installed system and not to the user. The system
//! itself lives outside the data directory and is never touched.
//!
//! Safety rules: the directory must be an absolute path with at least two components and no `..`, and not a symbolic
//! link, so a mistaken argument cannot reach `/` or a top-level folder. Symbolic links inside the directory are removed
//! as links and never followed. The marker is deleted whether or not every entry could be removed, so a failure cannot
//! loop; the [`Report`] lists what could not be removed.

use std::fs;
use std::io;
use std::path::{Component, Path};

/// The marker file Settings writes to ask for an erase at the next start.
pub const PENDING: &str = "erase.pending";
/// The keychain folder, erased before everything else.
pub const KEYCHAIN_DIR: &str = "keychain";
/// Entries of the data directory that belong to the installed system and survive an erase.
pub const KEEP: &[&str] = &["springboard.new", "springboard.part", "install-failures.log"];

/// What an erase did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Top-level entries removed.
    pub removed: usize,
    /// Top-level entries kept because they belong to the system.
    pub kept: usize,
    /// Entries that could not be removed, with the reason.
    pub failed: Vec<String>,
}

/// What an erase would do, for the confirmation text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Top-level entries to remove, keychain first, then by name.
    pub erase: Vec<String>,
    /// Top-level entries that stay.
    pub keep: Vec<String>,
    /// Bytes the erase frees (files counted to a depth of 16 folders).
    pub bytes: u64,
}

/// True when `dir` is a data directory that may be erased: absolute, two or more normal components, no `..`, and not
/// itself a symbolic link.
pub fn root_ok(dir: &Path) -> bool {
    if !dir.is_absolute() {
        return false;
    }
    let mut normal = 0;
    for c in dir.components() {
        match c {
            Component::Normal(_) => normal += 1,
            Component::RootDir => {}
            _ => return false,
        }
    }
    normal >= 2 && fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

fn refuse() -> io::Error {
    io::Error::other("erase: the data directory is not a safe place to erase")
}

/// Asks for an erase at the next start. Fails when the directory is not [`root_ok`].
pub fn request(dir: &Path) -> io::Result<()> {
    if !root_ok(dir) {
        return Err(refuse());
    }
    fs::write(dir.join(PENDING), b"erase all content and settings\n")
}

/// Withdraws a request. Returns whether there was one.
pub fn cancel(dir: &Path) -> io::Result<bool> {
    match fs::remove_file(dir.join(PENDING)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// True when a request is waiting. A marker that is a symbolic link or a folder does not count.
pub fn is_requested(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(PENDING)).is_ok_and(|m| m.is_file())
}

fn size_of(path: &Path, depth: u32) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() || depth == 0 {
        return 0;
    }
    fs::read_dir(path).into_iter().flatten().flatten().map(|e| size_of(&e.path(), depth - 1)).sum()
}

/// What an erase of `dir` would remove and keep.
pub fn plan(dir: &Path) -> io::Result<Plan> {
    if !root_ok(dir) {
        return Err(refuse());
    }
    let mut plan = Plan::default();
    for entry in fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == PENDING {
            continue;
        }
        if KEEP.contains(&name.as_str()) {
            plan.keep.push(name);
        } else {
            plan.bytes += size_of(&entry.path(), 16);
            plan.erase.push(name);
        }
    }
    plan.erase.sort();
    plan.keep.sort();
    if let Some(i) = plan.erase.iter().position(|n| n == KEYCHAIN_DIR) {
        let k = plan.erase.remove(i);
        plan.erase.insert(0, k);
    }
    Ok(plan)
}

fn remove_entry(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Erases the data directory now, keeping [`KEEP`], and removes the marker last. The caller must be sure the shell is not
/// running on this directory; [`apply_pending`] is the safe entry point.
pub fn execute(dir: &Path) -> io::Result<Report> {
    let plan = plan(dir)?;
    let mut report = Report { kept: plan.keep.len(), ..Report::default() };
    for name in &plan.erase {
        match remove_entry(&dir.join(name)) {
            Ok(()) => report.removed += 1,
            Err(e) => report.failed.push(format!("{name}: {e}")),
        }
    }
    let _ = fs::remove_file(dir.join(PENDING));
    Ok(report)
}

/// At start-up, before anything loads: erases when a request is waiting. `None` when there is none. A marker that is
/// not a plain file is removed and ignored.
pub fn apply_pending(dir: &Path) -> Option<io::Result<Report>> {
    let marker = dir.join(PENDING);
    if fs::symlink_metadata(&marker).is_err() {
        return None;
    }
    if !is_requested(dir) {
        let _ = remove_entry(&marker);
        return None;
    }
    Some(execute(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data directory with the usual contents, inside a temporary root.
    fn data() -> (std::path::PathBuf, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("var").join("data");
        fs::create_dir_all(dir.join("appdata/notes/Documents")).unwrap();
        fs::create_dir_all(dir.join("keychain")).unwrap();
        fs::write(dir.join("keychain/.master"), [7u8; 32]).unwrap();
        fs::write(dir.join("appdata/notes/Documents/a.txt"), b"private").unwrap();
        fs::write(dir.join("settings.ini"), b"volume=3\n").unwrap();
        fs::write(dir.join("springboard.new"), b"staged update").unwrap();
        fs::write(dir.join("install-failures.log"), b"log").unwrap();
        (dir, root)
    }

    #[test]
    fn unsafe_directories_are_refused() {
        assert!(!root_ok(Path::new("/")));
        assert!(!root_ok(Path::new("/var")));
        assert!(!root_ok(Path::new("relative/dir")));
        assert!(!root_ok(Path::new("/var/../etc")));
        assert!(!root_ok(Path::new("/definitely/not/a/real/dir")));
        assert!(request(Path::new("/")).is_err());
        assert!(execute(Path::new("/var")).is_err());
    }

    #[test]
    fn a_symbolic_link_is_not_a_data_directory() {
        let (dir, root) = data();
        let link = root.path().join("var").join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(root_ok(&dir));
        assert!(!root_ok(&link));
    }

    #[test]
    fn the_plan_lists_the_keychain_first_and_keeps_the_update_bookkeeping() {
        let (dir, _root) = data();
        let p = plan(&dir).unwrap();
        assert_eq!(p.erase, vec!["keychain", "appdata", "settings.ini"]);
        assert_eq!(p.keep, vec!["install-failures.log", "springboard.new"]);
        assert_eq!(p.bytes, 32 + "private".len() as u64 + "volume=3\n".len() as u64);
    }

    #[test]
    fn erasing_removes_user_data_and_keeps_the_system_files() {
        let (dir, _root) = data();
        request(&dir).unwrap();
        assert!(is_requested(&dir));
        let report = apply_pending(&dir).unwrap().unwrap();
        assert_eq!(report, Report { removed: 3, kept: 2, failed: vec![] });
        let mut left: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["install-failures.log", "springboard.new"]);
        assert!(!is_requested(&dir));
        assert!(apply_pending(&dir).is_none());
    }

    #[test]
    fn a_link_inside_the_directory_is_removed_and_its_target_is_left_alone() {
        let (dir, root) = data();
        let outside = root.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious.txt"), b"keep me").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("shortcut")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("appdata/notes/inner")).unwrap();
        request(&dir).unwrap();
        apply_pending(&dir).unwrap().unwrap();
        assert!(!dir.join("shortcut").exists());
        assert_eq!(fs::read(outside.join("precious.txt")).unwrap(), b"keep me");
    }

    #[test]
    fn nothing_happens_without_a_request() {
        let (dir, _root) = data();
        assert!(apply_pending(&dir).is_none());
        assert!(dir.join("settings.ini").exists());
    }

    #[test]
    fn a_withdrawn_request_erases_nothing() {
        let (dir, _root) = data();
        request(&dir).unwrap();
        assert!(cancel(&dir).unwrap());
        assert!(!cancel(&dir).unwrap());
        assert!(apply_pending(&dir).is_none());
        assert!(dir.join("settings.ini").exists());
    }

    #[test]
    fn a_marker_that_is_a_link_is_ignored_and_removed() {
        let (dir, root) = data();
        let target = root.path().join("elsewhere");
        fs::write(&target, b"x").unwrap();
        std::os::unix::fs::symlink(&target, dir.join(PENDING)).unwrap();
        assert!(!is_requested(&dir));
        assert!(apply_pending(&dir).is_none());
        assert!(fs::symlink_metadata(dir.join(PENDING)).is_err());
        assert!(target.exists() && dir.join("settings.ini").exists());
    }
}
