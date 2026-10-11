//! Relationships between records, with delete rules and count limits (`Relationship(deleteRule:minimumModelCount:
//! maximumModelCount:inverse:)`). Pure: it keeps the links, checks the counts and plans deletes. The owning store
//! applies the plan to its records.
//!
//! A relation belongs to one owner entity and its name is unique in the model. Its delete rule says what happens
//! to its targets when the owner is deleted. A link is only ever removed by the owner's delete, or by a `Nullify`
//! rule that clears the target's reference back to the deleted owner. Other references are left as they are, and
//! callers must skip ids that no longer exist.

use std::collections::{BTreeMap, BTreeSet};

/// What a relation does to its targets when its owner is deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteRule {
    /// The targets are deleted too, and their own rules apply in turn.
    Cascade,
    /// The owner cannot be deleted while it has targets.
    Deny,
    /// The targets stay, and their references back to the deleted owner are cleared.
    Nullify,
    /// The targets stay and nothing else is changed.
    NoAction,
}

/// The declaration of one relation: its owner entity, its name, its rule and how many targets it may hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub owner: String,
    pub name: String,
    pub rule: DeleteRule,
    pub min: usize,
    pub max: Option<usize>,
}

impl Relation {
    pub fn new(owner: &str, name: &str, rule: DeleteRule) -> Self {
        Relation { owner: owner.to_string(), name: name.to_string(), rule, min: 0, max: None }
    }
    /// The fewest targets a saved owner may hold.
    pub fn min(mut self, min: usize) -> Self {
        self.min = min;
        self
    }
    /// The most targets one owner may hold. Links past it are refused.
    pub fn max(mut self, max: usize) -> Self {
        self.max = Some(max);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelationError {
    /// No relation with this name was declared.
    Unknown(String),
    /// Linking would exceed the relation's maximum count.
    TooMany { relation: String, max: usize },
    /// The owner holds fewer targets than the relation's minimum.
    TooFew { relation: String, min: usize },
    /// The delete rule refuses to delete the owner while it has targets.
    Denied { relation: String, owner: u64 },
}

/// The links between records, keyed by owner id and relation name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relations {
    decls: BTreeMap<String, Relation>,
    links: BTreeMap<(u64, String), BTreeSet<u64>>,
}

impl Relations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a relation. Declaring the same name again replaces its rule and counts.
    pub fn declare(&mut self, relation: Relation) {
        self.decls.insert(relation.name.clone(), relation);
    }

    pub fn declaration(&self, name: &str) -> Option<&Relation> {
        self.decls.get(name)
    }

    /// Adds `target` to the owner's relation. Returns `false` when the link already exists.
    pub fn link(&mut self, owner: u64, relation: &str, target: u64) -> Result<bool, RelationError> {
        let max = self.decls.get(relation).ok_or_else(|| RelationError::Unknown(relation.to_string()))?.max;
        let key = (owner, relation.to_string());
        let held = self.links.get(&key);
        if held.is_some_and(|set| set.contains(&target)) {
            return Ok(false);
        }
        let count = held.map_or(0, BTreeSet::len);
        if let Some(max) = max {
            if count >= max {
                return Err(RelationError::TooMany { relation: relation.to_string(), max });
            }
        }
        self.links.entry(key).or_default().insert(target);
        Ok(true)
    }

    /// Removes one link. Returns whether it existed.
    pub fn unlink(&mut self, owner: u64, relation: &str, target: u64) -> bool {
        let key = (owner, relation.to_string());
        let Some(set) = self.links.get_mut(&key) else { return false };
        let removed = set.remove(&target);
        if set.is_empty() {
            self.links.remove(&key);
        }
        removed
    }

    /// The owner's targets for one relation, in id order.
    pub fn targets(&self, owner: u64, relation: &str) -> Vec<u64> {
        self.links.get(&(owner, relation.to_string())).map_or_else(Vec::new, |set| set.iter().copied().collect())
    }

    /// The inverse lookup: every (owner, relation) that points at `target`.
    pub fn referrers(&self, target: u64) -> Vec<(u64, String)> {
        self.links
            .iter()
            .filter(|(_, set)| set.contains(&target))
            .map(|((owner, relation), _)| (*owner, relation.clone()))
            .collect()
    }

    /// Checks the minimum counts of every relation that `entity` declares, for the record `owner`.
    pub fn check_counts(&self, owner: u64, entity: &str) -> Result<(), RelationError> {
        for decl in self.decls.values().filter(|d| d.owner == entity) {
            let held = self.links.get(&(owner, decl.name.clone())).map_or(0, BTreeSet::len);
            if held < decl.min {
                return Err(RelationError::TooFew { relation: decl.name.clone(), min: decl.min });
            }
        }
        Ok(())
    }

    /// Plans the delete of `root`: the ids to remove, root first in id order, including cascaded targets.
    /// Nothing is changed. A `Deny` rule that applies returns an error instead.
    pub fn plan_delete(&self, root: u64) -> Result<Vec<u64>, RelationError> {
        let mut doomed = BTreeSet::new();
        let mut queue = vec![root];
        while let Some(id) = queue.pop() {
            if !doomed.insert(id) {
                continue;
            }
            for ((owner, relation), targets) in &self.links {
                if *owner != id {
                    continue;
                }
                match self.rule_of(relation) {
                    Some(DeleteRule::Deny) => {
                        return Err(RelationError::Denied { relation: relation.clone(), owner: id });
                    }
                    Some(DeleteRule::Cascade) => queue.extend(targets.iter().copied()),
                    _ => {}
                }
            }
        }
        Ok(doomed.into_iter().collect())
    }

    /// Applies a delete of the given ids: their own links go, and `Nullify` rules clear the targets' references back
    /// to them. Other references to them stay until their owners change.
    pub fn apply_delete(&mut self, doomed: &[u64]) {
        let gone: BTreeSet<u64> = doomed.iter().copied().collect();
        let mut cleared: Vec<(u64, u64)> = Vec::new();
        for ((owner, relation), targets) in &self.links {
            if gone.contains(owner) && self.rule_of(relation) == Some(DeleteRule::Nullify) {
                cleared.extend(targets.iter().map(|target| (*target, *owner)));
            }
        }
        for (target, deleted) in cleared {
            for (key, set) in self.links.iter_mut() {
                if key.0 == target {
                    set.remove(&deleted);
                }
            }
        }
        self.links.retain(|(owner, _), _| !gone.contains(owner));
        self.links.retain(|_, set| !set.is_empty());
    }

    /// The owners whose links differ from `saved`, so a history can record them.
    pub fn changed_owners(&self, saved: &Relations) -> BTreeSet<u64> {
        let mut owners = BTreeSet::new();
        for (key, set) in &self.links {
            if saved.links.get(key) != Some(set) {
                owners.insert(key.0);
            }
        }
        for key in saved.links.keys() {
            if !self.links.contains_key(key) {
                owners.insert(key.0);
            }
        }
        owners
    }

    fn rule_of(&self, relation: &str) -> Option<DeleteRule> {
        self.decls.get(relation).map(|d| d.rule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_refuses_past_the_maximum_and_keeps_existing_links() {
        let mut r = Relations::new();
        r.declare(Relation::new("Folder", "pinned", DeleteRule::NoAction).max(1));
        assert_eq!(r.link(1, "pinned", 2), Ok(true));
        assert_eq!(r.link(1, "pinned", 3), Err(RelationError::TooMany { relation: "pinned".into(), max: 1 }));
        assert_eq!(r.link(1, "pinned", 2), Ok(false), "a repeated link is not a second target");
        assert_eq!(r.targets(1, "pinned"), vec![2]);
    }

    #[test]
    fn link_refuses_an_undeclared_relation() {
        let mut r = Relations::new();
        assert_eq!(r.link(1, "nope", 2), Err(RelationError::Unknown("nope".into())));
        assert!(r.targets(1, "nope").is_empty());
    }

    #[test]
    fn check_counts_reports_a_relation_below_its_minimum() {
        let mut r = Relations::new();
        r.declare(Relation::new("Playlist", "tracks", DeleteRule::NoAction).min(1));
        assert_eq!(r.check_counts(5, "Playlist"), Err(RelationError::TooFew { relation: "tracks".into(), min: 1 }));
        assert_eq!(r.check_counts(5, "Album"), Ok(()), "another entity's relations are not checked");
        r.link(5, "tracks", 6).unwrap();
        assert_eq!(r.check_counts(5, "Playlist"), Ok(()));
    }

    #[test]
    fn plan_delete_cascades_through_targets_and_lists_root_first() {
        let mut r = Relations::new();
        r.declare(Relation::new("Folder", "notes", DeleteRule::Cascade));
        r.declare(Relation::new("Note", "tags", DeleteRule::Cascade));
        r.link(1, "notes", 2).unwrap();
        r.link(1, "notes", 3).unwrap();
        r.link(3, "tags", 4).unwrap();
        assert_eq!(r.plan_delete(1), Ok(vec![1, 2, 3, 4]));
        assert_eq!(r.plan_delete(2), Ok(vec![2]), "a leaf has nothing to cascade");
    }

    #[test]
    fn deny_refuses_the_delete_and_changes_nothing() {
        let mut r = Relations::new();
        r.declare(Relation::new("Owner", "pets", DeleteRule::Deny));
        r.link(7, "pets", 8).unwrap();
        assert_eq!(r.plan_delete(7), Err(RelationError::Denied { relation: "pets".into(), owner: 7 }));
        assert_eq!(r.targets(7, "pets"), vec![8]);
    }

    #[test]
    fn nullify_clears_the_targets_reference_back_to_the_deleted_owner() {
        let mut r = Relations::new();
        r.declare(Relation::new("Note", "tags", DeleteRule::Nullify));
        r.declare(Relation::new("Tag", "notes", DeleteRule::NoAction));
        r.link(2, "tags", 9).unwrap();
        r.link(9, "notes", 2).unwrap();
        let doomed = r.plan_delete(2).unwrap();
        r.apply_delete(&doomed);
        assert!(r.targets(9, "notes").is_empty(), "the tag no longer points at the deleted note");
        assert!(r.targets(2, "tags").is_empty(), "the note's own links go with it");
    }

    #[test]
    fn a_reference_no_rule_clears_is_left_for_the_inverse_lookup() {
        let mut r = Relations::new();
        r.declare(Relation::new("Tag", "notes", DeleteRule::NoAction));
        r.link(9, "notes", 2).unwrap();
        r.apply_delete(&[2]);
        assert_eq!(r.referrers(2), vec![(9, "notes".to_string())]);
        assert_eq!(r.changed_owners(&Relations::new()), BTreeSet::from([9]));
    }
}
