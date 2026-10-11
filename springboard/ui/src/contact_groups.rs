//! Contact groups with subgroups: a tree of named groups, each holding contact identifiers. A contact can sit in
//! several groups; a group's members can include everything below it. The model does not own the contacts; the
//! store that does calls `forget_contact` when one is deleted.

use std::collections::{BTreeMap, BTreeSet};

/// Most groups kept in one set.
pub const MAX_GROUPS: usize = 1000;
/// Longest group name, in characters.
pub const MAX_NAME: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownGroup,
    EmptyName,
    NameTooLong,
    TooManyGroups,
    /// Moving a group under itself or under one of its own subgroups.
    WouldCycle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: u64,
    pub name: String,
    /// The enclosing group, or `None` at the top level.
    pub parent: Option<u64>,
    members: BTreeSet<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct Groups {
    groups: BTreeMap<u64, Group>,
    /// The last identifier handed out; identifiers are never reused.
    last_id: u64,
}

impl Groups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Group> {
        self.groups.get(&id)
    }

    /// Adds a group under `parent` (or at the top level) and returns its identifier.
    pub fn add(&mut self, name: &str, parent: Option<u64>) -> Result<u64, Error> {
        let name = checked_name(name)?;
        if self.groups.len() >= MAX_GROUPS {
            return Err(Error::TooManyGroups);
        }
        if let Some(parent) = parent {
            self.require(parent)?;
        }
        self.last_id += 1;
        let id = self.last_id;
        self.groups.insert(id, Group { id, name, parent, members: BTreeSet::new() });
        Ok(id)
    }

    pub fn rename(&mut self, id: u64, name: &str) -> Result<(), Error> {
        let name = checked_name(name)?;
        self.group_mut(id)?.name = name;
        Ok(())
    }

    /// Moves a group under `parent`, or to the top level for `None`. A group cannot move under itself or under one
    /// of its own subgroups; a refused move changes nothing.
    pub fn move_to(&mut self, id: u64, parent: Option<u64>) -> Result<(), Error> {
        self.require(id)?;
        if let Some(parent) = parent {
            self.require(parent)?;
            if self.is_within(parent, id) {
                return Err(Error::WouldCycle);
            }
        }
        self.group_mut(id)?.parent = parent;
        Ok(())
    }

    /// Removes a group with all of its subgroups and their memberships. Returns the removed identifiers, sorted.
    pub fn remove(&mut self, id: u64) -> Result<Vec<u64>, Error> {
        self.require(id)?;
        let mut doomed = self.subtree(id);
        for group in &doomed {
            self.groups.remove(group);
        }
        doomed.sort_unstable();
        Ok(doomed)
    }

    /// Direct subgroups of a group, in identifier order.
    pub fn children(&self, id: u64) -> Vec<u64> {
        self.groups.values().filter(|g| g.parent == Some(id)).map(|g| g.id).collect()
    }

    /// Adds a contact to a group. Returns false when the contact was already a member.
    pub fn add_member(&mut self, group: u64, contact: u64) -> Result<bool, Error> {
        Ok(self.group_mut(group)?.members.insert(contact))
    }

    /// Removes a contact from a group. Returns false when the contact was not a member.
    pub fn remove_member(&mut self, group: u64, contact: u64) -> Result<bool, Error> {
        Ok(self.group_mut(group)?.members.remove(&contact))
    }

    /// The members of a group, sorted, each once. With `recursive`, members of its subgroups count too.
    pub fn members(&self, group: u64, recursive: bool) -> Result<Vec<u64>, Error> {
        self.require(group)?;
        let scope = if recursive { self.subtree(group) } else { vec![group] };
        let mut all = BTreeSet::new();
        for id in scope {
            if let Some(g) = self.groups.get(&id) {
                all.extend(g.members.iter().copied());
            }
        }
        Ok(all.into_iter().collect())
    }

    /// The groups that hold a contact directly, in identifier order.
    pub fn groups_of(&self, contact: u64) -> Vec<u64> {
        self.groups.values().filter(|g| g.members.contains(&contact)).map(|g| g.id).collect()
    }

    /// Removes a deleted contact from every group. Returns how many memberships went.
    pub fn forget_contact(&mut self, contact: u64) -> usize {
        self.groups.values_mut().map(|g| usize::from(g.members.remove(&contact))).sum()
    }

    /// The group, then its subgroups, then their subgroups (breadth first, each once).
    fn subtree(&self, root: u64) -> Vec<u64> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let kids = self.children(out[i]);
            out.extend(kids);
            i += 1;
        }
        out
    }

    /// True when `group` is `ancestor` or sits below it. Walks upward, bounded by the group count.
    fn is_within(&self, mut group: u64, ancestor: u64) -> bool {
        for _ in 0..=self.groups.len() {
            if group == ancestor {
                return true;
            }
            match self.groups.get(&group).and_then(|g| g.parent) {
                Some(parent) => group = parent,
                None => return false,
            }
        }
        false
    }

    fn require(&self, id: u64) -> Result<(), Error> {
        if self.groups.contains_key(&id) {
            Ok(())
        } else {
            Err(Error::UnknownGroup)
        }
    }

    fn group_mut(&mut self, id: u64) -> Result<&mut Group, Error> {
        self.groups.get_mut(&id).ok_or(Error::UnknownGroup)
    }
}

/// A trimmed, non-empty name of at most `MAX_NAME` characters.
fn checked_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if name.chars().count() > MAX_NAME {
        return Err(Error::NameTooLong);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_of_a_group_include_its_subgroups_once_each_when_recursive() {
        let mut g = Groups::new();
        let family = g.add("Family", None).unwrap();
        let kids = g.add("Kids", Some(family)).unwrap();
        g.add_member(kids, 1).unwrap();
        g.add_member(family, 2).unwrap();
        g.add_member(family, 1).unwrap();
        assert_eq!(g.members(family, false).unwrap(), vec![1, 2]);
        assert_eq!(g.members(kids, false).unwrap(), vec![1]);
        assert_eq!(g.members(family, true).unwrap(), vec![1, 2], "contact 1 is counted once");
        assert_eq!(g.groups_of(1), vec![family, kids]);
        assert_eq!(g.add_member(family, 2).unwrap(), false, "adding twice is not a second membership");
        assert_eq!(g.remove_member(family, 2).unwrap(), true);
        assert_eq!(g.remove_member(family, 2).unwrap(), false);
    }

    #[test]
    fn a_group_cannot_move_under_itself_or_its_own_subgroup() {
        let mut g = Groups::new();
        let a = g.add("A", None).unwrap();
        let b = g.add("B", Some(a)).unwrap();
        let c = g.add("C", Some(b)).unwrap();
        assert_eq!(g.move_to(a, Some(c)), Err(Error::WouldCycle));
        assert_eq!(g.move_to(b, Some(b)), Err(Error::WouldCycle));
        assert_eq!(g.get(a).unwrap().parent, None, "a refused move changes nothing");
        assert_eq!(g.get(c).unwrap().parent, Some(b));
        g.move_to(c, None).unwrap();
        assert_eq!(g.get(c).unwrap().parent, None);
        g.move_to(c, Some(a)).unwrap();
        assert_eq!(g.children(a), vec![b, c]);
    }

    #[test]
    fn removing_a_group_takes_its_subgroups_and_their_memberships() {
        let mut g = Groups::new();
        let top = g.add("Top", None).unwrap();
        let mid = g.add("Mid", Some(top)).unwrap();
        let low = g.add("Low", Some(mid)).unwrap();
        let other = g.add("Other", None).unwrap();
        g.add_member(low, 7).unwrap();
        g.add_member(other, 7).unwrap();
        assert_eq!(g.remove(mid).unwrap(), vec![mid, low]);
        assert!(g.get(low).is_none() && g.get(top).is_some());
        assert_eq!(g.groups_of(7), vec![other], "the removed memberships are gone, the other group keeps its own");
    }

    #[test]
    fn a_deleted_contact_leaves_every_group() {
        let mut g = Groups::new();
        let a = g.add("A", None).unwrap();
        let b = g.add("B", None).unwrap();
        g.add_member(a, 5).unwrap();
        g.add_member(b, 5).unwrap();
        g.add_member(b, 6).unwrap();
        assert_eq!(g.forget_contact(5), 2);
        assert!(g.groups_of(5).is_empty());
        assert_eq!(g.members(b, false).unwrap(), vec![6]);
        assert_eq!(g.forget_contact(5), 0);
    }

    #[test]
    fn names_are_trimmed_and_checked_and_unknown_groups_are_refused() {
        let mut g = Groups::new();
        assert_eq!(g.add("   ", None), Err(Error::EmptyName));
        assert_eq!(g.add(&"x".repeat(MAX_NAME + 1), None), Err(Error::NameTooLong));
        let id = g.add("  Work  ", None).unwrap();
        assert_eq!(g.get(id).unwrap().name, "Work");
        g.add(&"y".repeat(MAX_NAME), None).expect("the longest name is accepted");
        g.rename(id, " Office ").unwrap();
        assert_eq!(g.get(id).unwrap().name, "Office");
        assert_eq!(g.add("Orphan", Some(99)), Err(Error::UnknownGroup));
        assert_eq!(g.add_member(99, 1), Err(Error::UnknownGroup));
        assert_eq!(g.members(99, true), Err(Error::UnknownGroup));
        assert_eq!(g.rename(99, "x"), Err(Error::UnknownGroup));
        assert_eq!(g.remove(99), Err(Error::UnknownGroup));
    }

    #[test]
    fn identifiers_are_never_reused_after_a_removal() {
        let mut g = Groups::new();
        let first = g.add("First", None).unwrap();
        g.remove(first).unwrap();
        let second = g.add("Second", None).unwrap();
        assert!(second > first);
        assert_eq!(g.len(), 1);
        assert!(!g.is_empty());
    }
}
