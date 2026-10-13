//! User and group identity for apps: the real and effective IDs of this process, its supplementary groups, and the
//! local account and group records (`/etc/passwd` and `/etc/group`) parsed into typed records.
//!
//! The ID calls read the process's credentials and change nothing. The parsers are pure: they take the text of the
//! file and return the records it holds, skipping comments, blank lines, directory-service markers (`+` and `-`) and
//! any line that is not well formed. Only local files are read, so an account that lives in a directory service is
//! not visible here.

use std::ffi::c_int;
use std::io;

extern "C" {
    fn getuid() -> u32;
    fn geteuid() -> u32;
    fn getgid() -> u32;
    fn getegid() -> u32;
    fn getgroups(size: c_int, list: *mut u32) -> c_int;
}

/// The IDs of the running process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ids {
    /// The real user ID.
    pub uid: u32,
    /// The effective user ID, the one permission checks use.
    pub euid: u32,
    /// The real group ID.
    pub gid: u32,
    /// The effective group ID.
    pub egid: u32,
}

impl Ids {
    /// True when the effective user is the superuser (ID 0).
    pub fn is_root(&self) -> bool {
        self.euid == 0
    }
}

/// The IDs of this process. The calls cannot fail.
pub fn current() -> Ids {
    // SAFETY: the ID getters take no arguments, cannot fail and touch no memory.
    unsafe { Ids { uid: getuid(), euid: geteuid(), gid: getgid(), egid: getegid() } }
}

/// The supplementary groups of this process, in the order the system reports them.
pub fn supplementary_groups() -> io::Result<Vec<u32>> {
    // SAFETY: a zero size with a null list only asks for the count and writes nothing.
    let count = unsafe { getgroups(0, std::ptr::null_mut()) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut list = vec![0u32; count as usize];
    // SAFETY: `list` holds `count` entries, the size given to the call.
    let got = unsafe { getgroups(count, list.as_mut_ptr()) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    list.truncate(got as usize);
    Ok(list)
}

/// One account record from `/etc/passwd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    /// The primary group.
    pub gid: u32,
    /// The free-form full name and details field.
    pub gecos: String,
    pub home: String,
    pub shell: String,
}

/// One group record from `/etc/group`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    /// The member names listed on the record. The primary group of a user is not listed here unless it is.
    pub members: Vec<String>,
}

/// Parses the text of a passwd file. A line needs seven colon-separated fields with numeric IDs.
pub fn parse_passwd(text: &str) -> Vec<User> {
    text.lines().filter_map(parse_passwd_line).collect()
}

fn parse_passwd_line(line: &str) -> Option<User> {
    let line = line.trim();
    if skip_line(line) {
        return None;
    }
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 7 || f[0].is_empty() {
        return None;
    }
    Some(User {
        name: f[0].to_owned(),
        uid: f[2].parse().ok()?,
        gid: f[3].parse().ok()?,
        gecos: f[4].to_owned(),
        home: f[5].to_owned(),
        shell: f[6].to_owned(),
    })
}

/// Parses the text of a group file. A line needs four colon-separated fields; members are comma-separated.
pub fn parse_group(text: &str) -> Vec<Group> {
    text.lines().filter_map(parse_group_line).collect()
}

fn parse_group_line(line: &str) -> Option<Group> {
    let line = line.trim();
    if skip_line(line) {
        return None;
    }
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 4 || f[0].is_empty() {
        return None;
    }
    Some(Group {
        name: f[0].to_owned(),
        gid: f[2].parse().ok()?,
        members: f[3].split(',').map(str::trim).filter(|m| !m.is_empty()).map(str::to_owned).collect(),
    })
}

/// Comments, blank lines and directory-service markers carry no local record.
fn skip_line(line: &str) -> bool {
    line.is_empty() || line.starts_with('#') || line.starts_with('+') || line.starts_with('-')
}

/// The first account with this user ID.
pub fn user_by_uid(users: &[User], uid: u32) -> Option<&User> {
    users.iter().find(|u| u.uid == uid)
}

/// The account with this name.
pub fn user_by_name<'a>(users: &'a [User], name: &str) -> Option<&'a User> {
    users.iter().find(|u| u.name == name)
}

/// The first group with this group ID.
pub fn group_by_gid(groups: &[Group], gid: u32) -> Option<&Group> {
    groups.iter().find(|g| g.gid == gid)
}

/// The groups that list `name` as a member. This is the supplementary membership record; the primary group is
/// separate and is found with [`group_by_gid`].
pub fn groups_listing<'a>(groups: &'a [Group], name: &str) -> Vec<&'a Group> {
    groups.iter().filter(|g| g.members.iter().any(|m| m == name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
# system accounts
root:x:0:0:root:/root:/bin/bash
+:::::
olibuijr:x:1000:1000:Oli Buijr,,,:/home/olibuijr:/bin/zsh
broken:x:not-a-number:0::/:/bin/sh
short:x:5:5
";

    const GROUP: &str = "\
wheel:x:10:olibuijr,root
audio:x:29:olibuijr,
empty:x:99:
";

    #[test]
    fn passwd_lines_become_user_records_and_bad_lines_are_skipped() {
        let users = parse_passwd(PASSWD);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].name, "root");
        assert_eq!(users[0].uid, 0);
        assert_eq!(users[1].gecos, "Oli Buijr,,,");
        assert_eq!(users[1].home, "/home/olibuijr");
        assert_eq!(users[1].shell, "/bin/zsh");
    }

    #[test]
    fn group_lines_list_their_members_without_empty_names() {
        let groups = parse_group(GROUP);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].members, vec!["olibuijr".to_owned(), "root".to_owned()]);
        assert_eq!(groups[1].members, vec!["olibuijr".to_owned()]);
        assert!(groups[2].members.is_empty());
    }

    #[test]
    fn lookups_find_the_account_group_and_memberships() {
        let users = parse_passwd(PASSWD);
        let groups = parse_group(GROUP);
        assert_eq!(user_by_uid(&users, 1000).map(|u| u.name.as_str()), Some("olibuijr"));
        assert_eq!(user_by_name(&users, "root").map(|u| u.uid), Some(0));
        assert!(user_by_name(&users, "nobody").is_none());
        assert_eq!(group_by_gid(&groups, 29).map(|g| g.name.as_str()), Some("audio"));
        let member_of: Vec<&str> = groups_listing(&groups, "olibuijr").iter().map(|g| g.name.as_str()).collect();
        assert_eq!(member_of, vec!["wheel", "audio"]);
    }

    #[test]
    fn the_effective_user_of_a_root_process_is_root() {
        let ids = Ids { uid: 1000, euid: 0, gid: 1000, egid: 1000 };
        assert!(ids.is_root());
        assert!(!Ids { euid: 1000, ..ids }.is_root());
    }

    #[test]
    fn the_current_ids_are_real_numbers_and_the_groups_read() {
        let ids = current();
        assert!(supplementary_groups().is_ok());
        assert_eq!(ids.is_root(), ids.euid == 0);
    }
}
