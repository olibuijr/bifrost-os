//! A managed object context over in-memory records (`NSManagedObjectContext`): objects are inserted, edited and
//! deleted in a working copy, the context knows which objects changed since the last save, `save` makes the working
//! copy the saved state, and `rollback` throws the working changes away. Fetches run over the working copy with the
//! same [`FetchRequest`] the list views use.
//!
//! Each object has an [`ObjectId`] that stays the same for its whole life, so a list can keep its selection while
//! objects change. A change is classified against the last saved state: an object that is new is `Inserted`, one
//! that differs from its saved copy is `Updated`, and one that was saved and is no longer present is `Deleted`. An
//! object inserted and deleted before a save never shows up as a change.

use std::collections::BTreeMap;

use crate::fetch::{fetch_indices, FetchRequest, Record};

/// The identity of one object in a context. Never reused within a context.
pub type ObjectId = u64;

/// How an object differs from the last saved state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Inserted,
    Updated,
    Deleted,
}

/// Why a context operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    /// No object with this identity is in the working copy.
    NotFound(ObjectId),
}

/// A working copy of records and the last saved copy they are compared with.
#[derive(Clone, Debug)]
pub struct ManagedContext<R> {
    saved: BTreeMap<ObjectId, R>,
    working: BTreeMap<ObjectId, R>,
    next_id: ObjectId,
}

impl<R> Default for ManagedContext<R> {
    fn default() -> Self {
        ManagedContext { saved: BTreeMap::new(), working: BTreeMap::new(), next_id: 1 }
    }
}

impl<R: Clone + PartialEq + Record> ManagedContext<R> {
    /// An empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `record` to the working copy and returns its identity.
    pub fn insert(&mut self, record: R) -> ObjectId {
        let id = self.next_id;
        self.next_id += 1;
        self.working.insert(id, record);
        id
    }

    /// The working copy of the object with this identity.
    pub fn get(&self, id: ObjectId) -> Option<&R> {
        self.working.get(&id)
    }

    /// Edits the working copy of an object in place.
    pub fn update(&mut self, id: ObjectId, edit: impl FnOnce(&mut R)) -> Result<(), ContextError> {
        match self.working.get_mut(&id) {
            Some(record) => {
                edit(record);
                Ok(())
            }
            None => Err(ContextError::NotFound(id)),
        }
    }

    /// Removes an object from the working copy.
    pub fn delete(&mut self, id: ObjectId) -> Result<R, ContextError> {
        self.working.remove(&id).ok_or(ContextError::NotFound(id))
    }

    /// Whether anything differs from the last saved state.
    pub fn has_changes(&self) -> bool {
        !self.changes().is_empty()
    }

    /// Every object that differs from the last saved state, in identity order.
    pub fn changes(&self) -> Vec<(ObjectId, Change)> {
        let mut out = Vec::new();
        for (id, record) in &self.working {
            match self.saved.get(id) {
                None => out.push((*id, Change::Inserted)),
                Some(saved) if saved != record => out.push((*id, Change::Updated)),
                Some(_) => {}
            }
        }
        for id in self.saved.keys() {
            if !self.working.contains_key(id) {
                out.push((*id, Change::Deleted));
            }
        }
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Makes the working copy the saved state and returns the changes that were saved.
    pub fn save(&mut self) -> Vec<(ObjectId, Change)> {
        let changes = self.changes();
        self.saved = self.working.clone();
        changes
    }

    /// Discards the working changes: the working copy becomes the saved state again.
    pub fn rollback(&mut self) {
        self.working = self.saved.clone();
    }

    /// The working objects that match `request`, in the request's order, with their identities.
    pub fn fetch(&self, request: &FetchRequest) -> Vec<(ObjectId, R)> {
        let ids: Vec<ObjectId> = self.working.keys().copied().collect();
        let records: Vec<R> = ids.iter().map(|id| self.working[id].clone()).collect();
        fetch_indices(&records, request).into_iter().map(|i| (ids[i], records[i].clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Field, Predicate};

    #[derive(Clone, Debug, PartialEq)]
    struct Note {
        title: String,
        pages: i64,
    }

    impl Record for Note {
        fn field(&self, key: &str) -> Option<Field> {
            match key {
                "title" => Some(Field::Text(self.title.clone())),
                "pages" => Some(Field::Int(self.pages)),
                _ => None,
            }
        }
    }

    fn note(title: &str, pages: i64) -> Note {
        Note { title: title.into(), pages }
    }

    #[test]
    fn a_new_context_has_no_changes() {
        let c: ManagedContext<Note> = ManagedContext::new();
        assert!(!c.has_changes());
        assert!(c.changes().is_empty());
    }

    #[test]
    fn inserts_are_changes_until_saved() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("a", 1));
        assert_eq!(c.changes(), vec![(a, Change::Inserted)]);
        assert_eq!(c.save(), vec![(a, Change::Inserted)]);
        assert!(!c.has_changes());
        assert_eq!(c.get(a), Some(&note("a", 1)));
    }

    #[test]
    fn edits_are_updates_and_an_edit_back_to_saved_is_not_a_change() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("a", 1));
        c.save();
        c.update(a, |n| n.pages = 5).unwrap();
        assert_eq!(c.changes(), vec![(a, Change::Updated)]);
        c.update(a, |n| n.pages = 1).unwrap();
        assert!(!c.has_changes());
    }

    #[test]
    fn deletes_of_saved_objects_are_changes() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("a", 1));
        c.save();
        assert_eq!(c.delete(a), Ok(note("a", 1)));
        assert_eq!(c.changes(), vec![(a, Change::Deleted)]);
        assert_eq!(c.get(a), None);
    }

    #[test]
    fn an_insert_deleted_before_save_is_not_a_change() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("temp", 1));
        c.delete(a).unwrap();
        assert!(!c.has_changes());
        assert!(c.save().is_empty());
    }

    #[test]
    fn rollback_restores_the_saved_state() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("a", 1));
        c.save();
        c.update(a, |n| n.title = "changed".into()).unwrap();
        c.insert(note("b", 2));
        c.delete(a).unwrap();
        c.rollback();
        assert!(!c.has_changes());
        assert_eq!(c.get(a), Some(&note("a", 1)));
    }

    #[test]
    fn ids_are_never_reused() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("a", 1));
        c.delete(a).unwrap();
        let b = c.insert(note("b", 2));
        assert_ne!(a, b);
    }

    #[test]
    fn unknown_identities_are_refused() {
        let mut c: ManagedContext<Note> = ManagedContext::new();
        assert_eq!(c.update(9, |_| {}), Err(ContextError::NotFound(9)));
        assert_eq!(c.delete(9), Err(ContextError::NotFound(9)));
    }

    #[test]
    fn fetch_runs_over_the_working_copy_with_identities() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        let _b = c.insert(note("beta", 2));
        c.save();
        let d = c.insert(note("delta", 30));
        c.delete(a).unwrap();
        let request = FetchRequest::new()
            .filter(Predicate::compare("pages", crate::fetch::Op::Gt, Field::Int(1)))
            .sorted_by("pages", false);
        let found: Vec<ObjectId> = c.fetch(&request).into_iter().map(|(id, _)| id).collect();
        assert_eq!(found, vec![d, 2]);
    }
}
