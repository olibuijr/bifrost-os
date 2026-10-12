//! Change history for the contact list, kept while the app runs. Every saved edit is recorded as adds, updates and
//! deletes in the `contact_changes` store. The store numbers its own records, so this keeps the map between the app's
//! contact identifiers and the store's. The list can then show the contacts changed since the app started.

use springboard::shell::contactstore::Contact;
use std::collections::{BTreeMap, BTreeSet};
use bifrost_app::controls::contact_changes::{Change, Error, Op, Store, MAX_HISTORY};

pub struct History {
    store: Store<Contact>,
    /// Contact identifier in the app to the store's identifier, for contacts that exist now.
    app_to_store: BTreeMap<u64, u64>,
    /// Every store identifier ever handed out, back to the contact identifier it stood for.
    store_to_app: BTreeMap<u64, u64>,
    /// The store token when the app started; changes after it are the session's changes.
    base: u64,
}

impl History {
    /// Starts a history for the contacts loaded at launch. Those are not changes, so the token starts after them.
    pub fn new(contacts: &[Contact]) -> Self {
        let mut history = History {
            store: Store::new(MAX_HISTORY),
            app_to_store: BTreeMap::new(),
            store_to_app: BTreeMap::new(),
            base: 0,
        };
        // A file the store refuses leaves the history empty; the next save then records the contacts as added.
        let _ = history.record(&[], contacts);
        history.base = history.store.token();
        history
    }

    /// Records the change from `before` to `after` (both full lists). Unchanged contacts record nothing.
    pub fn record(&mut self, before: &[Contact], after: &[Contact]) -> Result<(), Error> {
        let mut ops = Vec::new();
        let mut added = Vec::new();
        for contact in after {
            match self.app_to_store.get(&contact.id) {
                None => {
                    ops.push(Op::Add(contact.clone()));
                    added.push(contact.id);
                }
                Some(&store_id) => {
                    let old = before.iter().find(|b| b.id == contact.id);
                    if old.is_some_and(|old| old != contact) {
                        ops.push(Op::Update(store_id, contact.clone()));
                    }
                }
            }
        }
        let deleted: Vec<u64> = before
            .iter()
            .map(|c| c.id)
            .filter(|id| !after.iter().any(|c| c.id == *id))
            .collect();
        for id in &deleted {
            if let Some(&store_id) = self.app_to_store.get(id) {
                ops.push(Op::Delete(store_id));
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let ids = self.store.save(ops, |c: &Contact| c.valid())?;
        for (app_id, store_id) in added.into_iter().zip(ids) {
            self.app_to_store.insert(app_id, store_id);
            self.store_to_app.insert(store_id, app_id);
        }
        for id in &deleted {
            self.app_to_store.remove(id);
        }
        Ok(())
    }

    /// The contact identifiers that were added or changed since the app started. A contact deleted later is left out.
    pub fn changed_this_session(&self) -> BTreeSet<u64> {
        match self.store.changes_since(self.base) {
            Ok(events) => events
                .into_iter()
                .filter(|e| !matches!(e.change, Change::Deleted(_)))
                .filter_map(|e| self.store_to_app.get(&e.id).copied())
                .filter(|id| self.app_to_store.contains_key(id))
                .collect(),
            Err(_) => BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: u64, name: &str) -> Contact {
        Contact { id, name: name.into(), ..Default::default() }
    }

    #[test]
    fn contacts_loaded_at_launch_are_not_session_changes() {
        let history = History::new(&[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        assert!(history.changed_this_session().is_empty());
    }

    #[test]
    fn edits_adds_and_deletes_are_recorded_by_contact_identifier() {
        let start = vec![contact(1, "Anna As"), contact(2, "Bo Berg")];
        let mut history = History::new(&start);

        let edited = vec![contact(1, "Anna Ase"), contact(2, "Bo Berg"), contact(3, "Cara Cole")];
        history.record(&start, &edited).unwrap();
        assert_eq!(history.changed_this_session(), BTreeSet::from([1, 3]), "an unchanged contact is not a change");

        let removed = vec![contact(1, "Anna Ase"), contact(3, "Cara Cole")];
        history.record(&edited, &removed).unwrap();
        assert_eq!(history.changed_this_session(), BTreeSet::from([1, 3]), "a deleted contact drops out");
    }

    #[test]
    fn an_identifier_reused_after_a_delete_is_a_new_record() {
        let start = vec![contact(1, "Anna As")];
        let mut history = History::new(&start);
        history.record(&start, &[]).unwrap();
        let reused = vec![contact(1, "Dan Dahl")];
        history.record(&[], &reused).unwrap();
        assert_eq!(history.changed_this_session(), BTreeSet::from([1]));
        let edited = vec![contact(1, "Dan Dale")];
        history.record(&reused, &edited).unwrap();
        assert_eq!(history.changed_this_session(), BTreeSet::from([1]));
    }

    #[test]
    fn a_refused_batch_records_nothing() {
        let start = vec![contact(1, "Anna As")];
        let mut history = History::new(&start);
        let bad = vec![contact(1, "Anna As"), Contact { id: 2, ..Default::default() }];
        assert!(history.record(&start, &bad).is_err());
        assert!(history.changed_this_session().is_empty());
    }
}
