//! Identity directory: the user and group records that decide who can sign in and which groups a user belongs to.
//!
//! A user has a numeric id, a name, a full name, a primary group, a home folder and a shell. A group has a numeric id,
//! a name and a member list of user names. The model is pure: [`Directory::load`] reads the text forms of the two
//! records (one line per user or group, colon-separated, like the system's own files) and [`Directory::to_passwd`] and
//! [`Directory::to_group`] write them back. Nothing here reads the host's files. Lookups follow the calls that answer
//! them: by name, by numeric id, and the groups a user belongs to with the primary group first.

use std::collections::{BTreeMap, BTreeSet};

/// Most users kept in one directory.
pub const MAX_USERS: usize = 10_000;
/// Most groups kept in one directory.
pub const MAX_GROUPS: usize = 10_000;
/// Longest user or group name, in bytes.
pub const MAX_NAME: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyName,
    NameTooLong,
    /// A name starts with a lowercase letter or `_` and holds only lowercase letters, digits, `_`, `-` and `.`.
    BadName,
    /// A field holds a colon, a line break, or (for the full name) a comma.
    BadField,
    DuplicateName,
    DuplicateId,
    UnknownUser,
    UnknownGroup,
    /// The group is the primary group of at least one user.
    GroupInUse,
    TooManyUsers,
    TooManyGroups,
    /// A record has the wrong number of fields or a bad number. Holds the 1-based line.
    Malformed(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub name: String,
    pub full_name: String,
    pub primary_group: u32,
    pub home: String,
    pub shell: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub gid: u32,
    pub name: String,
    members: BTreeSet<String>,
}

impl Group {
    /// The member user names, in name order.
    pub fn members(&self) -> Vec<&str> {
        self.members.iter().map(|m| m.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    users: BTreeMap<u32, User>,
    groups: BTreeMap<u32, Group>,
}

impl Directory {
    pub fn new() -> Directory {
        Directory::default()
    }

    /// Adds a group. Ids and names are unique.
    pub fn add_group(&mut self, gid: u32, name: &str) -> Result<(), Error> {
        check_name(name)?;
        if self.groups.len() >= MAX_GROUPS {
            return Err(Error::TooManyGroups);
        }
        if self.groups.contains_key(&gid) {
            return Err(Error::DuplicateId);
        }
        if self.group(name).is_some() {
            return Err(Error::DuplicateName);
        }
        self.groups.insert(gid, Group { gid, name: name.to_string(), members: BTreeSet::new() });
        Ok(())
    }

    /// Adds a user. The primary group must already exist, and ids and names are unique.
    pub fn add_user(&mut self, user: User) -> Result<(), Error> {
        check_name(&user.name)?;
        check_field(&user.full_name, true)?;
        check_field(&user.home, false)?;
        check_field(&user.shell, false)?;
        if self.users.len() >= MAX_USERS {
            return Err(Error::TooManyUsers);
        }
        if self.users.contains_key(&user.uid) {
            return Err(Error::DuplicateId);
        }
        if self.user(&user.name).is_some() {
            return Err(Error::DuplicateName);
        }
        if !self.groups.contains_key(&user.primary_group) {
            return Err(Error::UnknownGroup);
        }
        self.users.insert(user.uid, user);
        Ok(())
    }

    pub fn user(&self, name: &str) -> Option<&User> {
        self.users.values().find(|u| u.name == name)
    }

    pub fn user_by_id(&self, uid: u32) -> Option<&User> {
        self.users.get(&uid)
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.values().find(|g| g.name == name)
    }

    pub fn group_by_id(&self, gid: u32) -> Option<&Group> {
        self.groups.get(&gid)
    }

    /// Puts a user into a group. Adding a user who is already a member changes nothing.
    pub fn add_member(&mut self, gid: u32, user: &str) -> Result<(), Error> {
        if self.user(user).is_none() {
            return Err(Error::UnknownUser);
        }
        let group = self.groups.get_mut(&gid).ok_or(Error::UnknownGroup)?;
        group.members.insert(user.to_string());
        Ok(())
    }

    /// Takes a user out of a group. Returns whether the user was a member. The primary group is not a member list
    /// entry, so this does not change it.
    pub fn remove_member(&mut self, gid: u32, user: &str) -> Result<bool, Error> {
        let group = self.groups.get_mut(&gid).ok_or(Error::UnknownGroup)?;
        Ok(group.members.remove(user))
    }

    /// Removes a user and every membership the user had. Returns the removed record.
    pub fn remove_user(&mut self, name: &str) -> Result<User, Error> {
        let uid = self.user(name).ok_or(Error::UnknownUser)?.uid;
        let user = self.users.remove(&uid).ok_or(Error::UnknownUser)?;
        for group in self.groups.values_mut() {
            group.members.remove(name);
        }
        Ok(user)
    }

    /// Removes a group. Refused while it is the primary group of any user.
    pub fn remove_group(&mut self, name: &str) -> Result<Group, Error> {
        let gid = self.group(name).ok_or(Error::UnknownGroup)?.gid;
        if self.users.values().any(|u| u.primary_group == gid) {
            return Err(Error::GroupInUse);
        }
        self.groups.remove(&gid).ok_or(Error::UnknownGroup)
    }

    /// The group ids a user belongs to: the primary group first, then the other groups the user is a member of, in
    /// id order. The same order the system's group lookup for a login returns.
    pub fn groups_of(&self, name: &str) -> Result<Vec<u32>, Error> {
        let user = self.user(name).ok_or(Error::UnknownUser)?;
        let mut out = vec![user.primary_group];
        for group in self.groups.values() {
            if group.gid != user.primary_group && group.members.contains(name) {
                out.push(group.gid);
            }
        }
        Ok(out)
    }

    /// Whether the user belongs to the group, as the primary group or as a listed member.
    pub fn is_member(&self, name: &str, gid: u32) -> bool {
        match self.user(name) {
            None => false,
            Some(user) => {
                user.primary_group == gid || self.groups.get(&gid).map_or(false, |g| g.members.contains(name))
            }
        }
    }

    /// Reads a directory from the text of a passwd-style file and a group-style file. A passwd line is
    /// `name:password:uid:gid:full name:home:shell`; a group line is `name:password:gid:member,member`. The password
    /// field is read and ignored, because sign-in is handled elsewhere. Blank lines and `#` comments are skipped.
    pub fn load(passwd: &str, group: &str) -> Result<Directory, Error> {
        let mut dir = Directory::new();
        let mut memberships = Vec::new();
        for (line, text) in records(group) {
            let f: Vec<&str> = text.split(':').collect();
            if f.len() != 4 {
                return Err(Error::Malformed(line));
            }
            let gid = number(f[2], line)?;
            dir.add_group(gid, f[0])?;
            for member in f[3].split(',').filter(|m| !m.is_empty()) {
                memberships.push((gid, member));
            }
        }
        for (line, text) in records(passwd) {
            let f: Vec<&str> = text.split(':').collect();
            if f.len() != 7 {
                return Err(Error::Malformed(line));
            }
            let user = User {
                uid: number(f[2], line)?,
                name: f[0].to_string(),
                full_name: f[4].split(',').next().unwrap_or("").to_string(),
                primary_group: number(f[3], line)?,
                home: f[5].to_string(),
                shell: f[6].to_string(),
            };
            dir.add_user(user)?;
        }
        for (gid, member) in memberships {
            dir.add_member(gid, member)?;
        }
        Ok(dir)
    }

    /// The passwd-style text of the users, in id order. The password field is always `x`.
    pub fn to_passwd(&self) -> String {
        let mut out = String::new();
        for u in self.users.values() {
            out.push_str(&format!("{}:x:{}:{}:{}:{}:{}\n", u.name, u.uid, u.primary_group, u.full_name, u.home, u.shell));
        }
        out
    }

    /// The group-style text of the groups, in id order. The password field is always `x`.
    pub fn to_group(&self) -> String {
        let mut out = String::new();
        for g in self.groups.values() {
            let members: Vec<&str> = g.members.iter().map(|m| m.as_str()).collect();
            out.push_str(&format!("{}:x:{}:{}\n", g.name, g.gid, members.join(",")));
        }
        out
    }
}

/// The non-blank, non-comment lines of a record file, with their 1-based line numbers.
fn records(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim_end()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn number(field: &str, line: usize) -> Result<u32, Error> {
    field.parse::<u32>().map_err(|_| Error::Malformed(line))
}

fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if name.len() > MAX_NAME {
        return Err(Error::NameTooLong);
    }
    let ok = name.bytes().enumerate().all(|(i, b)| {
        if i == 0 {
            b.is_ascii_lowercase() || b == b'_'
        } else {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        }
    });
    if ok { Ok(()) } else { Err(Error::BadName) }
}

/// A field may not hold a colon or a line break, and the full name may not hold a comma either.
fn check_field(text: &str, full_name: bool) -> Result<(), Error> {
    let bad = text.contains(':') || text.contains('\n') || text.contains('\r') || (full_name && text.contains(','));
    if bad { Err(Error::BadField) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: u32, name: &str, primary: u32) -> User {
        User {
            uid,
            name: name.into(),
            full_name: format!("{name} Example"),
            primary_group: primary,
            home: format!("/home/{name}"),
            shell: "/bin/sh".into(),
        }
    }

    fn sample() -> Directory {
        let mut d = Directory::new();
        d.add_group(100, "users").unwrap();
        d.add_group(10, "wheel").unwrap();
        d.add_group(20, "audio").unwrap();
        d.add_user(user(1000, "ada", 100)).unwrap();
        d.add_user(user(1001, "grace", 100)).unwrap();
        d.add_member(10, "ada").unwrap();
        d.add_member(20, "ada").unwrap();
        d.add_member(20, "grace").unwrap();
        d
    }

    #[test]
    fn users_are_found_by_name_and_id() {
        let d = sample();
        assert_eq!(d.user("ada").map(|u| u.uid), Some(1000));
        assert_eq!(d.user_by_id(1001).map(|u| u.name.as_str()), Some("grace"));
        assert!(d.user("nobody").is_none());
        assert_eq!(d.group("audio").map(|g| g.gid), Some(20));
        assert_eq!(d.group_by_id(10).map(|g| g.members()), Some(vec!["ada"]));
    }

    #[test]
    fn duplicate_ids_and_names_and_bad_names_are_refused() {
        let mut d = sample();
        assert_eq!(d.add_user(user(1000, "bob", 100)), Err(Error::DuplicateId));
        assert_eq!(d.add_user(user(1002, "ada", 100)), Err(Error::DuplicateName));
        assert_eq!(d.add_group(30, "users"), Err(Error::DuplicateName));
        assert_eq!(d.add_group(100, "other"), Err(Error::DuplicateId));
        assert_eq!(d.add_group(30, "Admin"), Err(Error::BadName));
        assert_eq!(d.add_group(30, "1admin"), Err(Error::BadName));
        assert_eq!(d.add_group(30, ""), Err(Error::EmptyName));
        assert_eq!(d.add_group(30, &"a".repeat(MAX_NAME + 1)), Err(Error::NameTooLong));
        assert_eq!(d.add_group(30, "ok_name-1.2"), Ok(()));
    }

    #[test]
    fn a_user_needs_an_existing_primary_group_and_clean_fields() {
        let mut d = sample();
        assert_eq!(d.add_user(user(1002, "bob", 999)), Err(Error::UnknownGroup));
        let mut bad = user(1002, "bob", 100);
        bad.home = "/home/a:b".into();
        assert_eq!(d.add_user(bad), Err(Error::BadField));
        let mut comma = user(1002, "bob", 100);
        comma.full_name = "Bob, Jr".into();
        assert_eq!(d.add_user(comma), Err(Error::BadField));
        assert_eq!(d.add_member(10, "nobody"), Err(Error::UnknownUser));
        assert_eq!(d.add_member(999, "ada"), Err(Error::UnknownGroup));
    }

    #[test]
    fn groups_of_puts_the_primary_group_first_then_memberships_in_id_order() {
        let d = sample();
        // ada: primary 100, member of 10 and 20. Id order after the primary is 10 then 20.
        assert_eq!(d.groups_of("ada"), Ok(vec![100, 10, 20]));
        assert_eq!(d.groups_of("grace"), Ok(vec![100, 20]));
        assert_eq!(d.groups_of("nobody"), Err(Error::UnknownUser));
        assert!(d.is_member("ada", 10));
        assert!(d.is_member("grace", 100), "the primary group counts as membership");
        assert!(!d.is_member("grace", 10));
    }

    #[test]
    fn a_primary_group_cannot_be_removed_but_an_empty_one_can() {
        let mut d = sample();
        assert_eq!(d.remove_group("users").map(|g| g.gid), Err(Error::GroupInUse));
        assert_eq!(d.remove_group("wheel").map(|g| g.gid), Ok(10));
        assert!(d.group("wheel").is_none());
        assert_eq!(d.remove_group("wheel").map(|g| g.gid), Err(Error::UnknownGroup));
    }

    #[test]
    fn removing_a_user_drops_every_membership() {
        let mut d = sample();
        let removed = d.remove_user("ada").unwrap();
        assert_eq!(removed.uid, 1000);
        assert!(d.user("ada").is_none());
        assert!(d.group_by_id(10).unwrap().members().is_empty());
        assert_eq!(d.group_by_id(20).unwrap().members(), vec!["grace"]);
        assert_eq!(d.remove_user("ada"), Err(Error::UnknownUser));
        assert_eq!(d.remove_member(20, "grace"), Ok(true));
        assert_eq!(d.remove_member(20, "grace"), Ok(false));
    }

    #[test]
    fn text_records_round_trip_through_load_and_render() {
        let d = sample();
        let back = Directory::load(&d.to_passwd(), &d.to_group()).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.groups_of("ada"), Ok(vec![100, 10, 20]));
        assert!(d.to_passwd().starts_with("ada:x:1000:100:ada Example:/home/ada:/bin/sh\n"));
        assert!(d.to_group().contains("wheel:x:10:ada\n"));
    }

    #[test]
    fn load_skips_blank_lines_and_comments_and_reports_the_bad_line() {
        let passwd = "# users\n\nada:x:1000:100:Ada:/home/ada:/bin/sh\n";
        let group = "users:x:100:\n";
        let d = Directory::load(passwd, group).unwrap();
        assert_eq!(d.user("ada").map(|u| u.full_name.as_str()), Some("Ada"));

        let short = "ada:x:1000:100:Ada:/home/ada\n";
        assert_eq!(Directory::load(short, group), Err(Error::Malformed(1)));
        let bad_number = "# header\nada:x:abc:100:Ada:/home/ada:/bin/sh\n";
        assert_eq!(Directory::load(bad_number, group), Err(Error::Malformed(2)));
        let unknown_member = "users:x:100:ghost\n";
        assert_eq!(Directory::load(passwd, unknown_member), Err(Error::UnknownUser));
    }

    #[test]
    fn capacity_limits_are_enforced() {
        let mut d = Directory::new();
        d.add_group(1, "staff").unwrap();
        for i in 0..MAX_USERS as u32 {
            d.users.insert(i + 1, user(i + 1, "x", 1));
        }
        assert_eq!(d.add_user(user(u32::MAX, "late", 1)), Err(Error::TooManyUsers));
    }
}
