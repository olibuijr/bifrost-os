//! A managed object context over in-memory records (`NSManagedObjectContext`): objects are inserted, edited and
//! deleted in a working copy, the context knows which objects changed since the last save, `save` makes the working
//! copy the saved state, and `rollback` throws the working changes away. Fetches run over the working copy with the
//! same [`FetchRequest`] the list views use.
//!
//! Each object has an [`ObjectId`] that stays the same for its whole life, so a list can keep its selection while
//! objects change. A change is classified against the last saved state: an object that is new is `Inserted`, one
//! that differs from its saved copy is `Updated`, and one that was saved and is no longer present is `Deleted`. An
//! object inserted and deleted before a save never shows up as a change.

use std::collections::{BTreeMap, BTreeSet};

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
    /// The saved state: the store that faulted objects are read from.
    saved: BTreeMap<ObjectId, R>,
    /// Resident working copies. An object is in exactly one of `working` or `faults`, unless it was deleted.
    working: BTreeMap<ObjectId, R>,
    /// Saved objects whose values are not in memory (faults). Each is also in `saved` and never in `working`.
    faults: BTreeSet<ObjectId>,
    next_id: ObjectId,
}

impl<R> Default for ManagedContext<R> {
    fn default() -> Self {
        ManagedContext { saved: BTreeMap::new(), working: BTreeMap::new(), faults: BTreeSet::new(), next_id: 1 }
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

    /// The resident working copy of the object with this identity. A fault has no resident copy, so this is `None`
    /// for it; [`ManagedContext::fault_in`] loads it.
    pub fn get(&self, id: ObjectId) -> Option<&R> {
        self.working.get(&id)
    }

    /// Whether the object is a fault: it exists, but its values are not in memory.
    pub fn is_fault(&self, id: ObjectId) -> bool {
        self.faults.contains(&id)
    }

    /// Loads a fault from the saved state (`NSManagedObject` faulting in). An object that is resident already is
    /// returned as it is.
    pub fn fault_in(&mut self, id: ObjectId) -> Result<&R, ContextError> {
        if self.faults.remove(&id) {
            let record = self.saved.get(&id).cloned().ok_or(ContextError::NotFound(id))?;
            self.working.insert(id, record);
        }
        self.working.get(&id).ok_or(ContextError::NotFound(id))
    }

    /// Releases the values of a saved object that has no unsaved change, leaving a fault. Returns false, and keeps
    /// the object resident, when it has changed since the last save or is not a saved object.
    pub fn fault_out(&mut self, id: ObjectId) -> bool {
        if self.faults.contains(&id) {
            return true;
        }
        let unchanged = match (self.working.get(&id), self.saved.get(&id)) {
            (Some(working), Some(saved)) => working == saved,
            _ => false,
        };
        if unchanged {
            self.working.remove(&id);
            self.faults.insert(id);
        }
        unchanged
    }

    /// Edits an object in place. A fault is loaded first.
    pub fn update(&mut self, id: ObjectId, edit: impl FnOnce(&mut R)) -> Result<(), ContextError> {
        self.fault_in(id)?;
        match self.working.get_mut(&id) {
            Some(record) => {
                edit(record);
                Ok(())
            }
            None => Err(ContextError::NotFound(id)),
        }
    }

    /// Removes an object from the context. A fault is removed without being loaded, and its saved copy is returned.
    pub fn delete(&mut self, id: ObjectId) -> Result<R, ContextError> {
        if self.faults.remove(&id) {
            return self.saved.get(&id).cloned().ok_or(ContextError::NotFound(id));
        }
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
            if !self.working.contains_key(id) && !self.faults.contains(id) {
                out.push((*id, Change::Deleted));
            }
        }
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Makes the working copy the saved state and returns the changes that were saved.
    pub fn save(&mut self) -> Vec<(ObjectId, Change)> {
        let changes = self.changes();
        // Faults keep their saved values, so they stay in the saved state.
        let mut next = self.working.clone();
        for id in &self.faults {
            if let Some(record) = self.saved.get(id) {
                next.insert(*id, record.clone());
            }
        }
        self.saved = next;
        changes
    }

    /// Discards the working changes: the working copy becomes the saved state again.
    pub fn rollback(&mut self) {
        self.working = self.saved.clone();
        self.faults.clear();
    }

    /// The saved state in identity order. A persistent store writes this after [`ManagedContext::save`].
    pub fn saved_state(&self) -> Vec<(ObjectId, R)> {
        self.saved.iter().map(|(id, record)| (*id, record.clone())).collect()
    }

    /// The identity the next insert will get. Identities are never reused, so a store keeps this with the saved state.
    pub fn next_id(&self) -> ObjectId {
        self.next_id
    }

    /// A context whose saved state is `saved`, with no working changes, as a store reads it back. `next_id` is raised
    /// past every saved identity, so an identity is never handed out twice.
    pub fn restored(next_id: ObjectId, saved: Vec<(ObjectId, R)>) -> Self {
        let saved: BTreeMap<ObjectId, R> = saved.into_iter().collect();
        let floor = saved.keys().next_back().map_or(1, |id| id + 1);
        ManagedContext {
            working: saved.clone(),
            saved,
            faults: BTreeSet::new(),
            next_id: next_id.max(floor),
        }
    }

    /// The working objects that match `request`, in the request's order, with their identities.
    pub fn fetch(&self, request: &FetchRequest) -> Vec<(ObjectId, R)> {
        // Faults are read from the saved state, so a fetch sees every object without loading it.
        let mut all = self.working.clone();
        for id in &self.faults {
            if let Some(record) = self.saved.get(id) {
                all.insert(*id, record.clone());
            }
        }
        let ids: Vec<ObjectId> = all.keys().copied().collect();
        let records: Vec<R> = ids.iter().map(|id| all[id].clone()).collect();
        fetch_indices(&records, request).into_iter().map(|i| (ids[i], records[i].clone())).collect()
    }
}

/// Live results over a context (`NSFetchedResultsController`, the list a `@FetchRequest` keeps current): a fetch
/// request and the rows it returned last time. `refresh` runs the request again over the context and reports whether
/// the rows changed (an identity, a value or the order), so a view redraws only when it has to.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedResults<R> {
    request: FetchRequest,
    items: Vec<(ObjectId, R)>,
}

impl<R: Clone + PartialEq + Record> FetchedResults<R> {
    /// Runs `request` over `context` now.
    pub fn new(request: FetchRequest, context: &ManagedContext<R>) -> Self {
        let items = context.fetch(&request);
        Self { request, items }
    }

    /// The rows as of the last run, in the request's order.
    pub fn items(&self) -> &[(ObjectId, R)] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Runs the request again over `context`. Returns true when the rows differ from the previous run.
    pub fn refresh(&mut self, context: &ManagedContext<R>) -> bool {
        let next = context.fetch(&self.request);
        let changed = next != self.items;
        self.items = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Field, Op, Predicate};

    #[test]
    fn live_results_refresh_only_when_the_rows_change() {
        let mut c = ManagedContext::new();
        c.insert(note("beta", 2));
        c.insert(note("alpha", 1));
        c.save();
        let mut live = FetchedResults::new(FetchRequest::new().sorted_by("title", true), &c);
        assert_eq!(live.len(), 2);
        assert_eq!(live.items()[0].1, note("alpha", 1));
        assert!(!live.refresh(&c), "nothing changed, so no redraw");
        let gamma = c.insert(note("gamma", 3));
        assert!(live.refresh(&c), "an insert adds a row");
        assert_eq!(live.len(), 3);
        c.update(gamma, |n| n.title = "aardvark".into()).unwrap();
        assert!(live.refresh(&c), "a value that moves a row re-orders the results");
        assert_eq!(live.items()[0].1, note("aardvark", 3));
        c.delete(gamma).unwrap();
        assert!(live.refresh(&c));
        assert_eq!(live.len(), 2);
        assert!(!live.is_empty());
    }

    #[test]
    fn live_results_keep_the_request_filter_and_limit() {
        let mut c = ManagedContext::new();
        c.insert(note("one", 1));
        c.insert(note("two", 9));
        c.save();
        let request = FetchRequest::new().filter(Predicate::compare("pages", Op::Gt, Field::Int(5))).limit(1);
        let mut live = FetchedResults::new(request, &c);
        assert_eq!(live.items().iter().map(|(_, n)| n.title.as_str()).collect::<Vec<_>>(), vec!["two"]);
        c.insert(note("three", 7));
        assert!(!live.refresh(&c), "the new row is cut off by the limit, so the rows do not change");
        assert_eq!(live.len(), 1, "the limit still applies after a refresh");
    }


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

    #[test]
    fn a_saved_unchanged_object_can_be_faulted_out_and_loaded_again() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        c.save();
        assert!(c.fault_out(a));
        assert!(c.is_fault(a));
        assert_eq!(c.get(a), None, "a fault has no resident copy");
        assert!(!c.has_changes(), "a fault is not a change");
        assert_eq!(c.fault_in(a).unwrap(), &note("alpha", 10));
        assert!(!c.is_fault(a));
    }

    #[test]
    fn a_changed_object_is_not_faulted_out() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        c.save();
        c.update(a, |n| n.pages = 11).unwrap();
        assert!(!c.fault_out(a));
        assert!(!c.is_fault(a));
        assert_eq!(c.get(a), Some(&note("alpha", 11)));
    }

    #[test]
    fn faults_are_saved_and_fetched_without_loading() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        let b = c.insert(note("beta", 2));
        c.save();
        assert!(c.fault_out(a));
        c.save();
        assert!(c.is_fault(a), "saving keeps a fault a fault");
        let request = FetchRequest::new().sorted_by("pages", true);
        let found: Vec<(ObjectId, Note)> = c.fetch(&request);
        assert_eq!(found, vec![(b, note("beta", 2)), (a, note("alpha", 10))]);
        assert!(c.is_fault(a), "a fetch does not load faults");
    }

    #[test]
    fn updating_a_fault_loads_it_and_records_an_update() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        c.save();
        c.fault_out(a);
        c.update(a, |n| n.pages = 12).unwrap();
        assert!(!c.is_fault(a));
        assert_eq!(c.changes(), vec![(a, Change::Updated)]);
    }

    #[test]
    fn deleting_a_fault_is_a_delete_change_and_returns_the_saved_copy() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        c.save();
        c.fault_out(a);
        assert_eq!(c.delete(a), Ok(note("alpha", 10)));
        assert_eq!(c.changes(), vec![(a, Change::Deleted)]);
    }

    #[test]
    fn rollback_loads_every_fault_back_to_the_saved_state() {
        let mut c = ManagedContext::new();
        let a = c.insert(note("alpha", 10));
        c.save();
        c.fault_out(a);
        c.rollback();
        assert!(!c.is_fault(a));
        assert_eq!(c.get(a), Some(&note("alpha", 10)));
    }
}
