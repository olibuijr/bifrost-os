//! User and group identity for apps: the process's real and effective IDs, its supplementary groups, and the local
//! account and group records. This is the `getuid`/`getpwuid`/`getgrgid` side of the system's identity services.
//!
//! ```ignore
//! let ids = identity::current();                // ids.uid, ids.euid, ids.is_root()
//! let me = identity::current_user()?;           // Option<User>: name, home, shell
//! let member_of = identity::groups_of(&me.name)?; // groups that list the user as a member
//! ```
//! Records come from the local `/etc/passwd` and `/etc/group`; accounts that a directory service provides are not
//! listed. The calls read the files on each call and change nothing. The parsing is in `bifrost_core::identity`.
use std::fs;
use std::io;

pub use bifrost_core::identity::{
    current, group_by_gid, groups_listing, parse_group, parse_passwd, supplementary_groups, user_by_name, user_by_uid,
    Group, Ids, User,
};

/// The account file the lookups read.
pub const PASSWD_FILE: &str = "/etc/passwd";
/// The group file the lookups read.
pub const GROUP_FILE: &str = "/etc/group";

/// Every local account.
pub fn users() -> io::Result<Vec<User>> {
    Ok(parse_passwd(&fs::read_to_string(PASSWD_FILE)?))
}

/// Every local group.
pub fn groups() -> io::Result<Vec<Group>> {
    Ok(parse_group(&fs::read_to_string(GROUP_FILE)?))
}

/// The account of the running process's real user, if it has a local record.
pub fn current_user() -> io::Result<Option<User>> {
    let uid = current().uid;
    Ok(user_by_uid(&users()?, uid).cloned())
}

/// The groups that list `name` as a member.
pub fn groups_of(name: &str) -> io::Result<Vec<Group>> {
    Ok(groups_listing(&groups()?, name).into_iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_user_id_matches_the_kernel_status_record() {
        // The status file lists the real, effective, saved and filesystem user IDs on its `Uid:` line.
        let status = fs::read_to_string("/proc/self/status").unwrap();
        let line = status.lines().find(|l| l.starts_with("Uid:")).unwrap();
        let real: u32 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert_eq!(current().uid, real);
    }

    #[test]
    fn the_root_account_is_listed_when_the_account_file_exists() {
        if let Ok(list) = users() {
            assert_eq!(user_by_name(&list, "root").map(|u| u.uid), Some(0));
        }
    }
}
