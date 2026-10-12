//! Index maintenance: named searchable indexes, the client delegate the system calls to rebuild an index, and the
//! index extension that runs one rebuild request against that delegate when the app is not running.
//!
//! [`IndexDescription`] names an index and caps its item count. [`IndexStore`] holds the item identifiers of one index
//! and enforces the cap and the item rules; a batch is all or nothing. [`IndexClientDelegate`] is what an app
//! implements to rebuild its items. [`IndexExtension`] runs one [`Request`] against the delegate and always returns an
//! [`Acknowledgement`]: the return value takes the place of the completion handler, so every request gets exactly one.
//!
//! Pure model, std only. Gaps: nothing is wired into the shell or the SDK yet, so no system process starts the
//! extension; there are no data protection classes, no data-for-item or file-URL requests, and the store keeps
//! identifiers only, not item content.

use std::collections::BTreeSet;
use std::fmt;

/// The largest item cap an index description may ask for.
pub const MAX_ITEM_LIMIT: usize = 100_000;

/// Why an index operation failed. The codes are stable: logs and tests may rely on [`IndexError::code`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexError {
    /// The named index is not known to this app, or the indexer is not running.
    IndexUnavailable,
    /// Indexing is not supported on this device.
    IndexingUnsupported,
    /// The index description or client state is invalid.
    InvalidClientState,
    /// An item identifier is empty, so the item cannot be stored.
    InvalidItem,
    /// The client state did not match what the index holds.
    MismatchedClientState,
    /// Adding the items would take the index past its item cap.
    QuotaExceeded,
    /// The request could not reach the process that performs it.
    RemoteConnection,
    /// Any other failure, reported by a delegate.
    Unknown,
}

impl IndexError {
    /// The error domain name, for logs.
    pub const DOMAIN: &'static str = "bifrost.index";

    /// The stable numeric code within [`IndexError::DOMAIN`].
    pub fn code(self) -> u32 {
        match self {
            IndexError::IndexUnavailable => 1,
            IndexError::IndexingUnsupported => 2,
            IndexError::InvalidClientState => 3,
            IndexError::InvalidItem => 4,
            IndexError::MismatchedClientState => 5,
            IndexError::QuotaExceeded => 6,
            IndexError::RemoteConnection => 7,
            IndexError::Unknown => 8,
        }
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            IndexError::IndexUnavailable => "the index is unavailable",
            IndexError::IndexingUnsupported => "indexing is not supported on this device",
            IndexError::InvalidClientState => "the index description is invalid",
            IndexError::InvalidItem => "an item identifier is empty",
            IndexError::MismatchedClientState => "the client state does not match the index",
            IndexError::QuotaExceeded => "the index is full",
            IndexError::RemoteConnection => "the indexer could not be reached",
            IndexError::Unknown => "an unknown indexing error occurred",
        };
        write!(f, "{} ({} {})", text, IndexError::DOMAIN, self.code())
    }
}

impl std::error::Error for IndexError {}

/// A named index: its name and the most items it may hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexDescription {
    name: String,
    item_limit: usize,
}

impl IndexDescription {
    /// The item cap the shell's per-app index uses.
    pub const DEFAULT_ITEM_LIMIT: usize = 1000;

    /// A description for an index called `name` holding at most `item_limit` items. The name must be non-empty,
    /// must not contain `/` (it is part of the storage key), and the limit must be 1 to [`MAX_ITEM_LIMIT`].
    pub fn new(name: &str, item_limit: usize) -> Result<Self, IndexError> {
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return Err(IndexError::InvalidClientState);
        }
        if item_limit == 0 || item_limit > MAX_ITEM_LIMIT {
            return Err(IndexError::InvalidClientState);
        }
        Ok(IndexDescription { name: name.to_string(), item_limit })
    }

    /// The index name, trimmed.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The most items the index may hold.
    pub fn item_limit(&self) -> usize {
        self.item_limit
    }

    /// The storage key for this index under an app's bundle identifier: `bundle/name`.
    pub fn storage_key(&self, bundle_id: &str) -> String {
        format!("{}/{}", bundle_id, self.name)
    }
}

/// The item identifiers held by one index, kept within the index's item cap.
#[derive(Clone, Debug)]
pub struct IndexStore {
    description: IndexDescription,
    items: BTreeSet<String>,
}

impl IndexStore {
    /// An empty store for `description`.
    pub fn new(description: IndexDescription) -> Self {
        IndexStore { description, items: BTreeSet::new() }
    }

    /// The description this store was created with.
    pub fn description(&self) -> &IndexDescription {
        &self.description
    }

    /// The number of items held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the index holds no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether an item with this identifier is held.
    pub fn contains(&self, identifier: &str) -> bool {
        self.items.contains(identifier)
    }

    /// Adds or replaces the items named by `identifiers`. An identifier already held is replaced and does not count
    /// against the cap. The batch is checked first: an empty identifier fails with [`IndexError::InvalidItem`], and
    /// a batch that would take the index past its cap fails with [`IndexError::QuotaExceeded`]. Either way nothing is
    /// stored. Returns the item count after the batch.
    pub fn index(&mut self, identifiers: &[&str]) -> Result<usize, IndexError> {
        if identifiers.iter().any(|id| id.is_empty()) {
            return Err(IndexError::InvalidItem);
        }
        let fresh: BTreeSet<&str> = identifiers.iter().copied().filter(|id| !self.items.contains(*id)).collect();
        if self.items.len() + fresh.len() > self.description.item_limit() {
            return Err(IndexError::QuotaExceeded);
        }
        for id in fresh {
            self.items.insert(id.to_string());
        }
        Ok(self.items.len())
    }

    /// Removes the items named by `identifiers` and returns how many were held and removed. Unknown identifiers are
    /// ignored.
    pub fn delete(&mut self, identifiers: &[&str]) -> usize {
        identifiers.iter().filter(|id| self.items.remove(**id)).count()
    }

    /// Removes every item and returns how many were held.
    pub fn delete_all(&mut self) -> usize {
        let n = self.items.len();
        self.items.clear();
        n
    }
}

/// What a rebuild request asks the delegate to redo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Rebuild every item of the index.
    All,
    /// Rebuild only the items with these identifiers.
    Items(Vec<String>),
}

/// The outcome of one request. The system waits for exactly one of these per request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acknowledgement {
    pub index: String,
    pub request: Request,
    pub outcome: Result<(), IndexError>,
}

/// What an app implements so the system can ask it to rebuild an index. Each method returns the outcome that the
/// acknowledgement reports.
pub trait IndexClientDelegate {
    /// Rebuild every item of `index`.
    fn reindex_all(&mut self, index: &IndexDescription) -> Result<(), IndexError>;
    /// Rebuild the items named by `identifiers` in `index`.
    fn reindex_items(&mut self, index: &IndexDescription, identifiers: &[String]) -> Result<(), IndexError>;
}

/// The maintenance extension: it knows which indexes the app owns and runs rebuild requests against the delegate.
#[derive(Clone, Debug)]
pub struct IndexExtension {
    indexes: Vec<IndexDescription>,
    acknowledged: u32,
}

impl IndexExtension {
    /// An extension that owns `indexes`.
    pub fn new(indexes: Vec<IndexDescription>) -> Self {
        IndexExtension { indexes, acknowledged: 0 }
    }

    /// Runs `request` for the index called `index`. An unknown index is acknowledged with
    /// [`IndexError::IndexUnavailable`] and the delegate is not called. Every call returns one acknowledgement.
    pub fn handle<D: IndexClientDelegate>(&mut self, delegate: &mut D, index: &str, request: Request) -> Acknowledgement {
        let outcome = match self.indexes.iter().find(|d| d.name() == index) {
            None => Err(IndexError::IndexUnavailable),
            Some(desc) => match &request {
                Request::All => delegate.reindex_all(desc),
                Request::Items(ids) => delegate.reindex_items(desc, ids),
            },
        };
        self.acknowledged += 1;
        Acknowledgement { index: index.to_string(), request, outcome }
    }

    /// How many requests this extension has acknowledged.
    pub fn acknowledged(&self) -> u32 {
        self.acknowledged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_rejects_empty_name_slash_and_bad_limits() {
        assert_eq!(IndexDescription::new("  ", 10), Err(IndexError::InvalidClientState));
        assert_eq!(IndexDescription::new("a/b", 10), Err(IndexError::InvalidClientState));
        assert_eq!(IndexDescription::new("notes", 0), Err(IndexError::InvalidClientState));
        assert_eq!(IndexDescription::new("notes", MAX_ITEM_LIMIT + 1), Err(IndexError::InvalidClientState));
        let d = IndexDescription::new(" notes ", 5).unwrap();
        assert_eq!(d.name(), "notes");
        assert_eq!(d.storage_key("org.example.app"), "org.example.app/notes");
    }

    #[test]
    fn replacing_an_item_does_not_count_against_the_cap() {
        let mut s = IndexStore::new(IndexDescription::new("notes", 2).unwrap());
        assert_eq!(s.index(&["a", "b"]), Ok(2));
        // "c" is new and the index is full: the batch fails and "a" keeps its place.
        assert_eq!(s.index(&["c"]), Err(IndexError::QuotaExceeded));
        assert!(!s.contains("c"));
        // Re-indexing "a" replaces it and stays within the cap.
        assert_eq!(s.index(&["a"]), Ok(2));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn a_batch_with_an_empty_identifier_stores_nothing() {
        let mut s = IndexStore::new(IndexDescription::new("notes", 10).unwrap());
        assert_eq!(s.index(&["ok", ""]), Err(IndexError::InvalidItem));
        assert!(s.is_empty());
    }

    #[test]
    fn delete_counts_only_held_items() {
        let mut s = IndexStore::new(IndexDescription::new("notes", 10).unwrap());
        s.index(&["a", "b", "c"]).unwrap();
        assert_eq!(s.delete(&["a", "zzz", "c"]), 2);
        assert_eq!(s.len(), 1);
        assert_eq!(s.delete_all(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(IndexError::IndexUnavailable.code(), 1);
        assert_eq!(IndexError::InvalidItem.code(), 4);
        assert_eq!(IndexError::QuotaExceeded.code(), 6);
        assert_eq!(IndexError::Unknown.code(), 8);
        assert_eq!(IndexError::QuotaExceeded.to_string(), "the index is full (bifrost.index 6)");
    }

    struct Recorder {
        all_calls: u32,
        items_seen: Vec<String>,
        fail_with: Option<IndexError>,
    }

    impl IndexClientDelegate for Recorder {
        fn reindex_all(&mut self, _index: &IndexDescription) -> Result<(), IndexError> {
            self.all_calls += 1;
            match self.fail_with {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        fn reindex_items(&mut self, _index: &IndexDescription, identifiers: &[String]) -> Result<(), IndexError> {
            self.items_seen.extend(identifiers.iter().cloned());
            Ok(())
        }
    }

    #[test]
    fn extension_acknowledges_every_request_with_the_delegate_outcome() {
        let mut ext = IndexExtension::new(vec![IndexDescription::new("notes", 10).unwrap()]);
        let mut d = Recorder { all_calls: 0, items_seen: Vec::new(), fail_with: None };
        let ack = ext.handle(&mut d, "notes", Request::All);
        assert_eq!(ack.outcome, Ok(()));
        assert_eq!(d.all_calls, 1);

        d.fail_with = Some(IndexError::RemoteConnection);
        let ack = ext.handle(&mut d, "notes", Request::All);
        assert_eq!(ack.outcome, Err(IndexError::RemoteConnection));
        assert_eq!(ext.acknowledged(), 2);
    }

    #[test]
    fn unknown_index_is_acknowledged_without_calling_the_delegate() {
        let mut ext = IndexExtension::new(vec![IndexDescription::new("notes", 10).unwrap()]);
        let mut d = Recorder { all_calls: 0, items_seen: Vec::new(), fail_with: None };
        let ack = ext.handle(&mut d, "tasks", Request::All);
        assert_eq!(ack.outcome, Err(IndexError::IndexUnavailable));
        assert_eq!(ack.index, "tasks");
        assert_eq!(d.all_calls, 0);
        assert_eq!(ext.acknowledged(), 1);
    }

    #[test]
    fn item_requests_reach_the_delegate_with_their_identifiers() {
        let mut ext = IndexExtension::new(vec![IndexDescription::new("notes", 10).unwrap()]);
        let mut d = Recorder { all_calls: 0, items_seen: Vec::new(), fail_with: None };
        let ids = vec!["n1".to_string(), "n2".to_string()];
        let ack = ext.handle(&mut d, "notes", Request::Items(ids.clone()));
        assert_eq!(ack.outcome, Ok(()));
        assert_eq!(ack.request, Request::Items(ids));
        assert_eq!(d.items_seen, vec!["n1".to_string(), "n2".to_string()]);
        assert_eq!(d.all_calls, 0);
    }
}
