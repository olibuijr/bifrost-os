//! The local account directory: the users and groups of this machine, read from the system account tables.
//!
//! `/etc/passwd` and `/etc/group` are read line by line, which is what the `getpwent` and `getgrent` calls walk.
//! Each passwd line becomes a `users` record and each group line a `groups` record, with the standard POSIX
//! attribute names of an entry (`uid`, `uidNumber`, `gidNumber`, `cn`, `homeDirectory`, `loginShell`, and for a
//! group `memberUid`). Lines that are blank, commented, short or have a non-numeric id are skipped. A name that
//! appears twice keeps its first line, since a node refuses a second record with the same type and name.
//!
//! The parsing is pure and the file read is the only I/O, in [`read_system_tables`]. [`fill_node`] loads a node of a
//! [`Session`] from the two texts, so the directory can be tested with fixtures.

use crate::directory_node::Node;
use crate::directory_query::DirRecord;

/// The record type of a local user (the same type a contact search reads as people).
pub const USER_RECORD_TYPE: &str = "users";
/// The record type of a local group.
pub const GROUP_RECORD_TYPE: &str = "groups";
/// The system account table of users.
pub const PASSWD_PATH: &str = "/etc/passwd";
/// The system account table of groups.
pub const GROUP_PATH: &str = "/etc/group";

/// The users in passwd text, one record per usable line, in file order.
pub fn parse_passwd(text: &str) -> Vec<DirRecord> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // name:password:uid:gid:gecos:home:shell
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 7 || fields[0].is_empty() {
            continue;
        }
        let (Ok(uid), Ok(gid)) = (fields[2].parse::<u32>(), fields[3].parse::<u32>()) else {
            continue;
        };
        let mut record = DirRecord::new(USER_RECORD_TYPE, fields[0]);
        set(&mut record, "uid", vec![fields[0].to_string()]);
        set(&mut record, "uidNumber", vec![uid.to_string()]);
        set(&mut record, "gidNumber", vec![gid.to_string()]);
        // GECOS is a comma-separated list whose first field is the full name.
        let full_name = fields[4].split(',').next().unwrap_or("").trim();
        if !full_name.is_empty() {
            set(&mut record, "cn", vec![full_name.to_string()]);
        }
        set(&mut record, "homeDirectory", vec![fields[5].to_string()]);
        set(&mut record, "loginShell", vec![fields[6].to_string()]);
        out.push(record);
    }
    out
}

/// The groups in group text, one record per usable line, in file order. Members are the comma-separated names in
/// the fourth field.
pub fn parse_group(text: &str) -> Vec<DirRecord> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // name:password:gid:members
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 4 || fields[0].is_empty() {
            continue;
        }
        let Ok(gid) = fields[2].parse::<u32>() else {
            continue;
        };
        let mut record = DirRecord::new(GROUP_RECORD_TYPE, fields[0]);
        set(&mut record, "cn", vec![fields[0].to_string()]);
        set(&mut record, "gidNumber", vec![gid.to_string()]);
        let members: Vec<String> =
            fields[3].split(',').map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).collect();
        set(&mut record, "memberUid", members);
        out.push(record);
    }
    out
}

/// Adds the users of `passwd` and the groups of `group` to `node`. A name already in the node (a user and a group
/// may share a name; two users may not) keeps the record it has. Returns how many records were added.
pub fn fill_node(node: &mut Node, passwd: &str, group: &str) -> usize {
    let mut added = 0;
    for record in parse_passwd(passwd).into_iter().chain(parse_group(group)) {
        if node.add_record(record).is_ok() {
            added += 1;
        }
    }
    added
}

/// Reads `/etc/passwd` and `/etc/group`. A missing or unreadable table reads as empty text.
pub fn read_system_tables() -> (String, String) {
    let passwd = std::fs::read_to_string(PASSWD_PATH).unwrap_or_default();
    let group = std::fs::read_to_string(GROUP_PATH).unwrap_or_default();
    (passwd, group)
}

/// Sets an attribute whose name is never the reserved record name and whose values are never empty here.
fn set(record: &mut DirRecord, attribute: &str, values: Vec<String>) {
    let _ = record.set_values(attribute, values);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_node::Session;

    const PASSWD: &str = "\
# system accounts
root:x:0:0:root:/root:/bin/bash
anna:x:1000:1000:Anna Berg,Office,555:/home/anna:/bin/zsh
broken:x:notanumber:0::/x:/bin/sh
short:x:5
root:x:9:9:again:/tmp:/bin/sh
";

    const GROUP: &str = "\
wheel:x:10:anna,root
staff:x:1000:
";

    #[test]
    fn passwd_lines_become_user_records_with_posix_attributes() {
        let users = parse_passwd(PASSWD);
        assert_eq!(users.len(), 3, "the comment, the bad id and the short line are skipped");
        let anna = users.iter().find(|r| r.name() == "anna").unwrap();
        assert_eq!(anna.record_type(), USER_RECORD_TYPE);
        assert_eq!(anna.values("uidNumber"), vec!["1000"]);
        assert_eq!(anna.values("gidNumber"), vec!["1000"]);
        assert_eq!(anna.values("cn"), vec!["Anna Berg"], "GECOS keeps only the full name");
        assert_eq!(anna.values("homeDirectory"), vec!["/home/anna"]);
        assert_eq!(anna.values("loginShell"), vec!["/bin/zsh"]);
    }

    #[test]
    fn group_members_are_split_and_an_empty_list_has_none() {
        let groups = parse_group(GROUP);
        assert_eq!(groups.len(), 2);
        let wheel = groups.iter().find(|r| r.name() == "wheel").unwrap();
        assert_eq!(wheel.record_type(), GROUP_RECORD_TYPE);
        assert_eq!(wheel.values("gidNumber"), vec!["10"]);
        assert_eq!(wheel.values("memberUid"), vec!["anna", "root"]);
        let staff = groups.iter().find(|r| r.name() == "staff").unwrap();
        assert!(staff.values("memberUid").is_empty());
    }

    #[test]
    fn fill_node_keeps_the_first_of_a_duplicate_name() {
        let mut session = Session::new();
        session.add_node("/Local").unwrap();
        let node = session.node_mut("/Local").unwrap();
        // root and anna are added; the second root is a duplicate. wheel and staff are added as groups.
        let added = fill_node(node, PASSWD, GROUP);
        assert_eq!(added, 4);
        assert_eq!(node.record("users", "root").unwrap().values("uidNumber"), vec!["0"]);
        assert!(node.record("groups", "wheel").is_some());
    }

    #[test]
    fn a_user_and_a_group_may_share_a_name() {
        let mut session = Session::new();
        session.add_node("/Local").unwrap();
        let node = session.node_mut("/Local").unwrap();
        fill_node(node, "staff:x:1:1::/:/bin/sh\n", "staff:x:1:\n");
        assert!(node.record("users", "staff").is_some());
        assert!(node.record("groups", "staff").is_some());
    }

    #[test]
    fn empty_tables_add_nothing() {
        let mut session = Session::new();
        session.add_node("/Local").unwrap();
        let node = session.node_mut("/Local").unwrap();
        assert_eq!(fill_node(node, "", ""), 0);
        assert_eq!(node.record_count(), 0);
    }
}
