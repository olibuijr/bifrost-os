//! Per-app, per-name search index stores: items with domains and expiration dates, a bounded size with oldest-first
//! eviction, batch client state, and queries that keep one app's items from another app unless the item is public.
//!
//! Pure model. The store is in memory; the shell writes one file per [`crate::searchindex::store_file_name`] from
//! these values. Time is passed in as Unix seconds so tests do not depend on the clock.

use crate::searchindex::{validate_name, IndexDescription, NameError, ProtectionClass};
use crate::searchquery::{run, Error as QueryError, Hit, Query, Record};
use std::collections::HashMap;

/// Most items one index keeps. Adding more drops the oldest first.
pub const MAX_ITEMS: usize = 1000;
/// Longest accepted client state, in bytes (`endBatch(withClientState:)`).
pub const MAX_CLIENT_STATE: usize = 250;

/// One stored item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredItem {
    pub unique_identifier: String,
    pub domain_identifier: String,
    pub record: Record,
    /// `expirationDate`, Unix seconds; the item is dropped once this time has passed.
    pub expiration: Option<i64>,
    /// A public item can be found by other apps; the default is private to the owner.
    pub public: bool,
}

impl StoredItem {
    pub fn new(unique_identifier: &str, domain_identifier: &str, record: Record) -> Self {
        StoredItem { unique_identifier: unique_identifier.into(), domain_identifier: domain_identifier.into(), record, expiration: None, public: false }
    }
    pub fn expiring(mut self, at: i64) -> Self {
        self.expiration = Some(at);
        self
    }
    pub fn public(mut self) -> Self {
        self.public = true;
        self
    }
    fn expired(&self, now: i64) -> bool {
        self.expiration.is_some_and(|at| at <= now)
    }
}

/// Why a batch call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchError {
    /// `endBatch` without `beginBatch`, or `beginBatch` while a batch is open.
    BatchState,
    /// The client state is longer than [`MAX_CLIENT_STATE`].
    ClientStateTooLong,
}

/// One index: the items, in the order they were added, and the last committed client state.
#[derive(Clone, Debug, Default)]
pub struct IndexStore {
    items: Vec<StoredItem>,
    client_state: Vec<u8>,
    in_batch: bool,
    protection: Option<ProtectionClass>,
}

impl IndexStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn protection(&self) -> Option<ProtectionClass> {
        self.protection
    }
    pub fn ids(&self) -> Vec<&str> {
        self.items.iter().map(|i| i.unique_identifier.as_str()).collect()
    }

    /// `indexSearchableItems`: an item with a known identifier replaces the old one and counts as newest.
    /// Over [`MAX_ITEMS`] the oldest items are dropped.
    pub fn index(&mut self, items: Vec<StoredItem>) {
        for item in items {
            self.items.retain(|i| i.unique_identifier != item.unique_identifier);
            self.items.push(item);
        }
        if self.items.len() > MAX_ITEMS {
            let extra = self.items.len() - MAX_ITEMS;
            self.items.drain(..extra);
        }
    }

    /// `deleteSearchableItems(withIdentifiers:)`.
    pub fn delete_identifiers(&mut self, ids: &[&str]) {
        self.items.retain(|i| !ids.contains(&i.unique_identifier.as_str()));
    }

    /// `deleteSearchableItems(withDomainIdentifiers:)`. A domain also matches its sub-domains: deleting `mail`
    /// removes `mail.inbox`, but not `mailbox`.
    pub fn delete_domains(&mut self, domains: &[&str]) {
        self.items.retain(|i| !domains.iter().any(|d| in_domain(&i.domain_identifier, d)));
    }

    /// `deleteAllSearchableItems`. The client state is kept.
    pub fn delete_all(&mut self) {
        self.items.clear();
    }

    /// Drops items whose expiration date is at or before `now`. Returns how many were dropped.
    pub fn purge_expired(&mut self, now: i64) -> usize {
        let before = self.items.len();
        self.items.retain(|i| !i.expired(now));
        before - self.items.len()
    }

    /// `beginBatch()`.
    pub fn begin_batch(&mut self) -> Result<(), BatchError> {
        if self.in_batch {
            return Err(BatchError::BatchState);
        }
        self.in_batch = true;
        Ok(())
    }

    /// `endBatch(withClientState:)`: closes the batch and keeps `state` as the last client state.
    pub fn end_batch(&mut self, state: &[u8]) -> Result<(), BatchError> {
        if !self.in_batch {
            return Err(BatchError::BatchState);
        }
        if state.len() > MAX_CLIENT_STATE {
            return Err(BatchError::ClientStateTooLong);
        }
        self.in_batch = false;
        self.client_state = state.to_vec();
        Ok(())
    }

    /// `fetchLastClientState`.
    pub fn client_state(&self) -> &[u8] {
        &self.client_state
    }

    /// The live items a requester may see: all of them for the owner, the public ones for anyone else.
    fn visible(&self, now: i64, owner: bool) -> Vec<&StoredItem> {
        self.items.iter().filter(|i| !i.expired(now) && (owner || i.public)).collect()
    }
}

/// `domain` is `parent` or a dotted sub-domain of it.
fn in_domain(domain: &str, parent: &str) -> bool {
    domain == parent || domain.strip_prefix(parent).is_some_and(|rest| rest.starts_with('.'))
}

/// A matching item and its score.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub unique_identifier: String,
    pub domain_identifier: String,
    pub score: u32,
}

/// All indexes, keyed by app and optional index name.
#[derive(Clone, Debug, Default)]
pub struct StoreSet {
    stores: HashMap<(String, Option<String>), IndexStore>,
}

impl StoreSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// The default index of `app`, created when missing.
    pub fn default_index(&mut self, app: &str) -> &mut IndexStore {
        self.stores.entry((app.to_string(), None)).or_default()
    }

    /// `CSSearchableIndex(name:protectionClass:)`: the named index of `app`, created when missing. The protection
    /// class of an existing index is kept.
    pub fn named_index(&mut self, app: &str, description: &IndexDescription) -> Result<&mut IndexStore, NameError> {
        validate_name(&description.name)?;
        let store = self.stores.entry((app.to_string(), Some(description.name.clone()))).or_default();
        if store.protection.is_none() {
            store.protection = description.protection_class;
        }
        Ok(store)
    }

    /// Removes a named index and its items. Returns whether it existed.
    pub fn delete_named_index(&mut self, app: &str, name: &str) -> bool {
        self.stores.remove(&(app.to_string(), Some(name.to_string()))).is_some()
    }

    /// The names of the named indexes of `app`, sorted.
    pub fn index_names(&self, app: &str) -> Vec<&str> {
        let mut names: Vec<&str> = self.stores.keys().filter_map(|(a, n)| if a.as_str() == app { n.as_deref() } else { None }).collect();
        names.sort_unstable();
        names
    }

    /// Runs `query` as `requester` over the index `name` of `owner`. The owner sees every live item; another app
    /// sees only public ones. A missing index gives no hits.
    pub fn search(&self, requester: &str, owner: &str, name: Option<&str>, query: &Query, now: i64) -> Result<Vec<Found>, QueryError> {
        let items = match self.stores.get(&(owner.to_string(), name.map(str::to_string))) {
            Some(store) => store.visible(now, requester == owner),
            None => Vec::new(),
        };
        let records: Vec<Record> = items.iter().map(|i| i.record.clone()).collect();
        let hits: Vec<Hit> = run(query, &records)?;
        Ok(hits
            .into_iter()
            .map(|h| Found { unique_identifier: items[h.index].unique_identifier.clone(), domain_identifier: items[h.index].domain_identifier.clone(), score: h.score })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(title: &str) -> Record {
        Record { title: title.into(), ..Record::default() }
    }
    fn item(id: &str, domain: &str, title: &str) -> StoredItem {
        StoredItem::new(id, domain, rec(title))
    }
    fn ids(found: Vec<Found>) -> Vec<String> {
        found.into_iter().map(|f| f.unique_identifier).collect()
    }

    #[test]
    fn another_app_sees_only_public_items() {
        let mut set = StoreSet::new();
        set.default_index("notes").index(vec![item("a", "d", "Milk"), item("b", "d", "Milk").public()]);
        let q = Query::new("milk");
        assert_eq!(ids(set.search("notes", "notes", None, &q, 0).unwrap()), ["a", "b"]);
        assert_eq!(ids(set.search("mail", "notes", None, &q, 0).unwrap()), ["b"]);
    }

    #[test]
    fn named_indexes_are_separate_and_can_be_deleted() {
        let mut set = StoreSet::new();
        let archive = IndexDescription::new("archive").unwrap().protection(ProtectionClass::Complete);
        set.named_index("notes", &archive).unwrap().index(vec![item("x", "d", "Old milk")]);
        set.default_index("notes").index(vec![item("y", "d", "New milk")]);
        let q = Query::new("milk");
        assert_eq!(ids(set.search("notes", "notes", Some("archive"), &q, 0).unwrap()), ["x"]);
        assert_eq!(ids(set.search("notes", "notes", None, &q, 0).unwrap()), ["y"]);
        assert_eq!(set.named_index("notes", &archive).unwrap().protection(), Some(ProtectionClass::Complete));
        assert_eq!(set.index_names("notes"), ["archive"]);
        assert!(set.delete_named_index("notes", "archive"));
        assert!(!set.delete_named_index("notes", "archive"));
        assert!(set.search("notes", "notes", Some("archive"), &q, 0).unwrap().is_empty());
    }

    #[test]
    fn a_bad_index_name_is_refused() {
        let mut set = StoreSet::new();
        let bad = IndexDescription { name: "a/b".into(), protection_class: None };
        assert_eq!(set.named_index("notes", &bad).err(), Some(NameError::BadCharacter('/')));
    }

    #[test]
    fn domain_delete_takes_sub_domains_but_not_look_alikes() {
        let mut s = IndexStore::new();
        s.index(vec![item("1", "mail", "a"), item("2", "mail.inbox", "b"), item("3", "mailbox", "c")]);
        s.delete_domains(&["mail"]);
        assert_eq!(s.ids(), ["3"]);
    }

    #[test]
    fn expired_items_are_purged_and_never_returned() {
        let mut set = StoreSet::new();
        set.default_index("notes").index(vec![item("a", "d", "Milk").expiring(100), item("b", "d", "Milk")]);
        let q = Query::new("milk");
        assert_eq!(ids(set.search("notes", "notes", None, &q, 99).unwrap()), ["a", "b"]);
        assert_eq!(ids(set.search("notes", "notes", None, &q, 100).unwrap()), ["b"]);
        assert_eq!(set.default_index("notes").purge_expired(100), 1);
        assert_eq!(set.default_index("notes").ids(), ["b"]);
    }

    #[test]
    fn reindexing_replaces_and_the_oldest_are_evicted_over_the_cap() {
        let mut s = IndexStore::new();
        s.index((0..MAX_ITEMS).map(|n| item(&n.to_string(), "d", "t")).collect());
        s.index(vec![item("0", "d", "t")]);
        assert_eq!(s.len(), MAX_ITEMS);
        s.index(vec![item("new", "d", "t")]);
        assert_eq!(s.len(), MAX_ITEMS);
        assert!(!s.ids().contains(&"1"));
        assert!(s.ids().contains(&"0"));
    }

    #[test]
    fn batches_keep_the_client_state_across_delete_all() {
        let mut s = IndexStore::new();
        assert_eq!(s.end_batch(b"x"), Err(BatchError::BatchState));
        s.begin_batch().unwrap();
        assert_eq!(s.begin_batch(), Err(BatchError::BatchState));
        assert_eq!(s.end_batch(&[0; MAX_CLIENT_STATE + 1]), Err(BatchError::ClientStateTooLong));
        s.end_batch(b"cursor-7").unwrap();
        s.delete_all();
        assert_eq!(s.client_state(), b"cursor-7");
    }
}
