//! File metadata change events: mode, owner, flags, extended attributes and access control lists.
//!
//! The model keeps an attribute snapshot per path and diffs two snapshots into the events a monitor would
//! report for the change: mode, owner and group (one event carries both), flags, extended attribute set and
//! delete, and access control list set or clear. It is pure: nothing reads the file system, and no shell path
//! produces these events yet. Stat and directory-listing records are not events and are not modelled here.

use std::collections::BTreeMap;

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
