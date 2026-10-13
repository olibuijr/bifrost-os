//! File metadata change events: mode, owner, flags, extended attributes and access control lists.
//!
//! The model keeps an attribute snapshot per path and diffs two snapshots into the events a monitor would
//! report for the change: mode, owner and group (one event carries both), flags, extended attribute set and
//! delete, and access control list set or clear. The diff is pure: it reads nothing, and no shell path
//! produces these events yet. Stat and directory-listing records are not events and are not modelled here.
//!
//! The second half of the module reads and writes real extended attributes (never following a link) and builds
//! backup exclusion on top: an item marked with [`BACKUP_EXCLUDED_XATTR`] is skipped, with everything below it,
//! by [`backup_items`], which is the walk a backup job uses.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::io;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

extern "C" {
    fn lgetxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: usize) -> isize;
    fn lsetxattr(path: *const c_char, name: *const c_char, value: *const c_void, size: usize, flags: c_int) -> c_int;
    fn lremovexattr(path: *const c_char, name: *const c_char) -> c_int;
    fn llistxattr(path: *const c_char, list: *mut c_char, size: usize) -> isize;
}

/// `ENODATA`: the attribute is not set.
const ENODATA: i32 = 61;

/// The attribute that marks an item as excluded from backup (`URLResourceValues.isExcludedFromBackup`).
pub const BACKUP_EXCLUDED_XATTR: &str = "user.bifrost.exclude-from-backup";

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL"))
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "attribute name contains a NUL"))
}

/// The value of extended attribute `name` on `path`, or `None` when it is not set.
pub fn xattr(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: both strings are live NUL-terminated buffers; a null buffer with size 0 asks for the length.
    let size = unsafe { lgetxattr(p.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return missing_or_error();
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` is a live buffer of the length passed.
    let got = unsafe { lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr() as *mut c_void, buf.len()) };
    if got < 0 {
        return missing_or_error();
    }
    buf.truncate(got as usize);
    Ok(Some(buf))
}

fn missing_or_error() -> io::Result<Option<Vec<u8>>> {
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(ENODATA) {
        Ok(None)
    } else {
        Err(e)
    }
}

/// Sets extended attribute `name` on `path`, replacing any value.
pub fn set_xattr(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: the strings and `value` are live for the call.
    let rc = unsafe { lsetxattr(p.as_ptr(), n.as_ptr(), value.as_ptr() as *const c_void, value.len(), 0) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Removes extended attribute `name`. An attribute that is not set is not an error.
pub fn remove_xattr(path: &Path, name: &str) -> io::Result<()> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: both strings are live NUL-terminated buffers.
    let rc = unsafe { lremovexattr(p.as_ptr(), n.as_ptr()) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(ENODATA) {
            return Err(e);
        }
    }
    Ok(())
}

/// The names of the extended attributes set on `path`, sorted.
pub fn list_xattrs(path: &Path) -> io::Result<Vec<String>> {
    let p = c_path(path)?;
    // SAFETY: a null buffer with size 0 asks for the length.
    let size = unsafe { llistxattr(p.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` is a live buffer of the length passed.
    let got = unsafe { llistxattr(p.as_ptr(), buf.as_mut_ptr() as *mut c_char, buf.len()) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(got as usize);
    let mut names: Vec<String> =
        buf.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    names.sort();
    Ok(names)
}

/// Copies every extended attribute of `from` onto `to`. Attributes the target refuses are skipped.
pub fn copy_xattrs(from: &Path, to: &Path) -> io::Result<()> {
    for name in list_xattrs(from)? {
        if let Some(value) = xattr(from, &name)? {
            let _ = set_xattr(to, &name, &value);
        }
    }
    Ok(())
}

/// `URLResourceValues.isExcludedFromBackup` setter: marks or unmarks `path`.
pub fn set_excluded_from_backup(path: &Path, excluded: bool) -> io::Result<()> {
    if excluded {
        set_xattr(path, BACKUP_EXCLUDED_XATTR, b"1")
    } else {
        remove_xattr(path, BACKUP_EXCLUDED_XATTR)
    }
}

/// `URLResourceValues.isExcludedFromBackup` getter. An item whose attributes cannot be read counts as included.
pub fn is_excluded_from_backup(path: &Path) -> bool {
    matches!(xattr(path, BACKUP_EXCLUDED_XATTR), Ok(Some(_)))
}

/// The items a backup of `root` covers: `root`'s descendants in depth-first order, names sorted within each level.
/// An excluded item is left out together with everything below it. Symbolic links are listed, never entered. An
/// excluded `root` gives an empty list.
pub fn backup_items(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if is_excluded_from_backup(root) {
        return Ok(out);
    }
    collect_backup_items(root, &mut out)?;
    Ok(out)
}

fn collect_backup_items(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        names.push(entry?.file_name());
    }
    names.sort();
    for name in names {
        let path = dir.join(name);
        if is_excluded_from_backup(&path) {
            continue;
        }
        out.push(path.clone());
        if std::fs::symlink_metadata(&path)?.file_type().is_dir() {
            collect_backup_items(&path, out)?;
        }
    }
    Ok(())
}

/// The attributes of one path that a metadata event can change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    /// Permission bits (the low twelve bits of the file mode; the file type cannot change).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub flags: u32,
    /// Extended attributes by name, sorted by name.
    pub xattrs: BTreeMap<String, Vec<u8>>,
    /// The access control list in its text form, or `None` when the path has none.
    pub acl: Option<String>,
}

/// One metadata change on one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataEvent {
    Mode { path: String, mode: u32 },
    /// Owner and group travel together, as the owner-setting event does.
    Owner { path: String, uid: u32, gid: u32 },
    Flags { path: String, flags: u32 },
    SetExtattr { path: String, name: String },
    DeleteExtattr { path: String, name: String },
    /// `acl` is `None` when the list is cleared.
    SetAcl { path: String, acl: Option<String> },
}

impl MetadataEvent {
    /// Stable lower-case name, used in logs and in the control socket.
    pub fn name(&self) -> &'static str {
        match self {
            MetadataEvent::Mode { .. } => "mode",
            MetadataEvent::Owner { .. } => "owner",
            MetadataEvent::Flags { .. } => "flags",
            MetadataEvent::SetExtattr { .. } => "set-extattr",
            MetadataEvent::DeleteExtattr { .. } => "delete-extattr",
            MetadataEvent::SetAcl { .. } => "set-acl",
        }
    }

    pub fn path(&self) -> &str {
        match self {
            MetadataEvent::Mode { path, .. }
            | MetadataEvent::Owner { path, .. }
            | MetadataEvent::Flags { path, .. }
            | MetadataEvent::SetExtattr { path, .. }
            | MetadataEvent::DeleteExtattr { path, .. }
            | MetadataEvent::SetAcl { path, .. } => path,
        }
    }
}

const PERMISSION_BITS: u32 = 0o7777;

/// The events that turn `before` into `after` for `path`, in a fixed order: mode, owner, flags, extended
/// attribute deletions (by name), extended attribute sets (by name), then the access control list. An unchanged
/// attribute produces no event.
pub fn diff(path: &str, before: &Attributes, after: &Attributes) -> Vec<MetadataEvent> {
    let mut events = Vec::new();
    if before.mode & PERMISSION_BITS != after.mode & PERMISSION_BITS {
        events.push(MetadataEvent::Mode { path: path.to_string(), mode: after.mode & PERMISSION_BITS });
    }
    if before.uid != after.uid || before.gid != after.gid {
        events.push(MetadataEvent::Owner { path: path.to_string(), uid: after.uid, gid: after.gid });
    }
    if before.flags != after.flags {
        events.push(MetadataEvent::Flags { path: path.to_string(), flags: after.flags });
    }
    for name in before.xattrs.keys().filter(|name| !after.xattrs.contains_key(*name)) {
        events.push(MetadataEvent::DeleteExtattr { path: path.to_string(), name: name.clone() });
    }
    for (name, value) in &after.xattrs {
        if before.xattrs.get(name) != Some(value) {
            events.push(MetadataEvent::SetExtattr { path: path.to_string(), name: name.clone() });
        }
    }
    if before.acl != after.acl {
        events.push(MetadataEvent::SetAcl { path: path.to_string(), acl: after.acl.clone() });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(mode: u32, uid: u32, gid: u32) -> Attributes {
        Attributes { mode, uid, gid, ..Attributes::default() }
    }

    fn with_xattr(mut a: Attributes, name: &str, value: &[u8]) -> Attributes {
        a.xattrs.insert(name.to_string(), value.to_vec());
        a
    }

    #[test]
    fn identical_snapshots_produce_no_events() {
        let a = with_xattr(file(0o100644, 1000, 100), "user.tag", b"x");
        assert!(diff("/home/u/a", &a, &a.clone()).is_empty());
    }

    #[test]
    fn mode_change_reports_permission_bits_only() {
        // 0o100644 is a regular file with mode 644; 0o100600 is the same file with mode 600.
        let before = file(0o100644, 1000, 100);
        let after = file(0o100600, 1000, 100);
        let events = diff("/home/u/a", &before, &after);
        assert_eq!(events, vec![MetadataEvent::Mode { path: "/home/u/a".into(), mode: 0o600 }]);
        assert_eq!(events[0].name(), "mode");
    }

    #[test]
    fn owner_change_carries_both_uid_and_gid() {
        let before = file(0o644, 1000, 100);
        let after = file(0o644, 0, 100);
        assert_eq!(
            diff("/etc/x", &before, &after),
            vec![MetadataEvent::Owner { path: "/etc/x".into(), uid: 0, gid: 100 }]
        );
    }

    #[test]
    fn xattr_sets_and_deletes_are_sorted_by_name() {
        let before = with_xattr(with_xattr(file(0o644, 1, 1), "user.b", b"2"), "user.gone", b"1");
        let after = with_xattr(with_xattr(file(0o644, 1, 1), "user.a", b"new"), "user.b", b"3");
        let events = diff("/f", &before, &after);
        assert_eq!(
            events,
            vec![
                MetadataEvent::DeleteExtattr { path: "/f".into(), name: "user.gone".into() },
                MetadataEvent::SetExtattr { path: "/f".into(), name: "user.a".into() },
                MetadataEvent::SetExtattr { path: "/f".into(), name: "user.b".into() },
            ]
        );
    }

    #[test]
    fn acl_clear_is_reported_as_set_with_no_value() {
        let mut before = file(0o644, 1, 1);
        before.acl = Some("user:bob:read".into());
        let after = file(0o644, 1, 1);
        assert_eq!(
            diff("/f", &before, &after),
            vec![MetadataEvent::SetAcl { path: "/f".into(), acl: None }]
        );
    }

    #[test]
    fn events_follow_the_fixed_order() {
        let before = file(0o644, 1, 1);
        let mut after = file(0o600, 2, 1);
        after.flags = 0x10;
        after.acl = Some("everyone:deny".into());
        let names: Vec<&str> = diff("/f", &before, &after).iter().map(|e| e.name()).collect();
        assert_eq!(names, vec!["mode", "owner", "flags", "set-acl"]);
    }
}
