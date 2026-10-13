//! Index completion results, errors and batches for Core Spotlight (`CSIndexError`, `beginBatch()`,
//! `endBatch(withClientState:completionHandler:)`, `fetchLastClientState(completionHandler:)`).
//!
//! The shell index is fire-and-forget, so the app keeps a [`SearchIndexLedger`]: a model of what it has indexed. The
//! ledger validates every call the way the shell would (attribute size, the per-app item cap) and answers with
//! `Result<(), CSIndexError>`, so a refused call is reported instead of silently dropping or evicting items. Only calls
//! the ledger accepts reach the shell. See docs/bifrost-ui.md "Core Search".

use crate::quicksearch::{CSSearchableIndex, CSSearchableItem, IndexError};
use crate::Ctx;

/// The shell keeps at most this many items per app. Over it, indexing fails with [`CSIndexErrorCode::QuotaExceeded`].
pub const MAX_INDEXED_ITEMS: usize = 1000;
/// The most text bytes one item may carry across title, description, keywords, custom values, authors and content type.
pub const MAX_ATTRIBUTE_BYTES: usize = 4096;
/// `endBatch(withClientState:)` takes at most this many bytes (Apple's limit is 250).
pub const MAX_CLIENT_STATE_BYTES: usize = 250;

/// `CSIndexError.Code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CSIndexErrorCode {
    /// The index service cannot be reached or was closed.
    IndexUnavailableError,
    /// An item is malformed or over the attribute limit.
    InvalidItemError,
    /// The client state is too large, or no batch is open.
    InvalidClientStateError,
    /// The connection to the shell failed.
    RemoteConnectionError,
    /// Indexing would exceed [`MAX_INDEXED_ITEMS`].
    QuotaExceeded,
}

impl CSIndexErrorCode {
    /// The `CSIndexError` raw value; stable for logs and tests.
    pub fn raw_value(self) -> i32 {
        match self {
            CSIndexErrorCode::IndexUnavailableError => -1000,
            CSIndexErrorCode::InvalidItemError => -1001,
            CSIndexErrorCode::InvalidClientStateError => -1002,
            CSIndexErrorCode::RemoteConnectionError => -1003,
            CSIndexErrorCode::QuotaExceeded => -1004,
        }
    }
}

/// `CSIndexError`: a code plus the position of the offending item, when one is to blame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CSIndexError {
    pub code: CSIndexErrorCode,
    pub item_position: Option<usize>,
}

impl CSIndexError {
    /// `CSIndexErrorDomain`.
    pub const DOMAIN: &'static str = "bifrost.quicksearch.index";

    fn new(code: CSIndexErrorCode) -> Self {
        CSIndexError { code, item_position: None }
    }
}

/// One change to the index, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexOp {
    Index(Vec<CSSearchableItem>),
    DeleteIdentifiers(Vec<String>),
    DeleteDomains(Vec<String>),
    DeleteAll,
}

fn attribute_bytes(item: &CSSearchableItem) -> usize {
    let a = &item.attribute_set;
    a.title.len()
        + a.display_name.len()
        + a.content_description.len()
        + a.content_type.len()
        + a.keywords.iter().map(String::len).sum::<usize>()
        + a.author_names.iter().map(String::len).sum::<usize>()
        + a.custom.iter().map(|c| c.key.len() + c.value.len()).sum::<usize>()
}

/// The app's model of its index: item identifiers with their domains, the last committed client state and an open batch.
#[derive(Clone, Debug, Default)]
pub struct SearchIndexLedger {
    items: Vec<(String, String)>,
    client_state: Vec<u8>,
    batch: Option<Batch>,
    closed: bool,
}

#[derive(Clone, Debug)]
struct Batch {
    base: Vec<(String, String)>,
    ops: Vec<IndexOp>,
    failure: Option<CSIndexError>,
}

impl SearchIndexLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the index service unreachable: every later call fails with `indexUnavailableError`.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Number of items the ledger believes are indexed.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, identifier: &str) -> bool {
        self.items.iter().any(|(id, _)| id == identifier)
    }

    /// `indexSearchableItems`: all or nothing. Returns the ops to send to the shell now, empty inside a batch.
    pub fn index(&mut self, items: &[CSSearchableItem]) -> Result<Vec<IndexOp>, CSIndexError> {
        self.guard()?;
        let result = self.try_index(items);
        self.settle(result, IndexOp::Index(items.to_vec()))
    }

    /// `deleteSearchableItems(withIdentifiers:)`.
    pub fn delete_identifiers(&mut self, identifiers: &[&str]) -> Result<Vec<IndexOp>, CSIndexError> {
        self.guard()?;
        self.items.retain(|(id, _)| !identifiers.contains(&id.as_str()));
        self.settle(Ok(()), IndexOp::DeleteIdentifiers(identifiers.iter().map(|s| s.to_string()).collect()))
    }

    /// `deleteSearchableItems(withDomainIdentifiers:)`.
    pub fn delete_domains(&mut self, domains: &[&str]) -> Result<Vec<IndexOp>, CSIndexError> {
        self.guard()?;
        self.items.retain(|(_, d)| !domains.contains(&d.as_str()));
        self.settle(Ok(()), IndexOp::DeleteDomains(domains.iter().map(|s| s.to_string()).collect()))
    }

    /// `deleteAllSearchableItems`.
    pub fn delete_all(&mut self) -> Result<Vec<IndexOp>, CSIndexError> {
        self.guard()?;
        self.items.clear();
        self.settle(Ok(()), IndexOp::DeleteAll)
    }

    /// `beginBatch()`. Calls until [`Self::end_batch`] are held back and commit together.
    pub fn begin_batch(&mut self) -> Result<(), CSIndexError> {
        self.guard()?;
        if self.batch.is_some() {
            return Err(CSIndexError::new(CSIndexErrorCode::InvalidClientStateError));
        }
        self.batch = Some(Batch { base: self.items.clone(), ops: Vec::new(), failure: None });
        Ok(())
    }

    /// `endBatch(withClientState:)`. Commits every held-back call and stores `client_state`, or, when any call in the
    /// batch failed, discards them all, restores the index and returns that first error. Returns the ops to send.
    pub fn end_batch(&mut self, client_state: &[u8]) -> Result<Vec<IndexOp>, CSIndexError> {
        self.guard()?;
        let Some(batch) = self.batch.take() else {
            return Err(CSIndexError::new(CSIndexErrorCode::InvalidClientStateError));
        };
        let too_big = client_state.len() > MAX_CLIENT_STATE_BYTES;
        let failure = batch.failure.or_else(|| too_big.then(|| CSIndexError::new(CSIndexErrorCode::InvalidClientStateError)));
        if let Some(e) = failure {
            self.items = batch.base;
            return Err(e);
        }
        self.client_state = client_state.to_vec();
        Ok(batch.ops)
    }

    /// `fetchLastClientState`: the state of the last committed batch, empty when none was committed.
    pub fn fetch_last_client_state(&self) -> Result<Vec<u8>, CSIndexError> {
        self.guard()?;
        Ok(self.client_state.clone())
    }

    fn guard(&self) -> Result<(), CSIndexError> {
        if self.closed {
            Err(CSIndexError::new(CSIndexErrorCode::IndexUnavailableError))
        } else {
            Ok(())
        }
    }

    fn try_index(&mut self, items: &[CSSearchableItem]) -> Result<(), CSIndexError> {
        for (pos, i) in items.iter().enumerate() {
            if IndexError::check(i).is_err() || attribute_bytes(i) > MAX_ATTRIBUTE_BYTES {
                return Err(CSIndexError { code: CSIndexErrorCode::InvalidItemError, item_position: Some(pos) });
            }
        }
        let mut next = self.items.clone();
        for i in items {
            let entry = (i.unique_identifier.clone(), i.domain_identifier.clone());
            match next.iter_mut().find(|(id, _)| *id == entry.0) {
                Some(existing) => *existing = entry,
                None => next.push(entry),
            }
        }
        if next.len() > MAX_INDEXED_ITEMS {
            return Err(CSIndexError::new(CSIndexErrorCode::QuotaExceeded));
        }
        self.items = next;
        Ok(())
    }

    /// Inside a batch the op is held back and a failure is remembered; outside it the op is returned to send.
    fn settle(&mut self, result: Result<(), CSIndexError>, op: IndexOp) -> Result<Vec<IndexOp>, CSIndexError> {
        match (&mut self.batch, result) {
            (Some(b), Ok(())) => {
                b.ops.push(op);
                Ok(Vec::new())
            }
            (Some(b), Err(e)) => {
                b.failure.get_or_insert(e);
                Err(e)
            }
            (None, Ok(())) => Ok(vec![op]),
            (None, Err(e)) => Err(e),
        }
    }
}

impl CSSearchableIndex {
    fn send_ops(&self, ctx: &Ctx, ops: Vec<IndexOp>) {
        for op in ops {
            match op {
                IndexOp::Index(items) => self.index_searchable_items(ctx, &items),
                IndexOp::DeleteIdentifiers(ids) => self.delete_searchable_items(ctx, &ids.iter().map(String::as_str).collect::<Vec<_>>()),
                IndexOp::DeleteDomains(ds) => self.delete_searchable_items_with_domain_identifiers(ctx, &ds.iter().map(String::as_str).collect::<Vec<_>>()),
                IndexOp::DeleteAll => self.delete_all_searchable_items(ctx),
            }
        }
    }

    /// `indexSearchableItems(_:completionHandler:)`. `completion` runs once with the outcome; nothing is sent on failure.
    pub fn index_searchable_items_with_completion(&self, ctx: &Ctx, ledger: &mut SearchIndexLedger, items: &[CSSearchableItem], completion: impl FnOnce(Result<(), CSIndexError>)) {
        let r = ledger.index(items).map(|ops| self.send_ops(ctx, ops));
        completion(r);
    }

    /// `deleteSearchableItems(withIdentifiers:completionHandler:)`.
    pub fn delete_searchable_items_with_completion(&self, ctx: &Ctx, ledger: &mut SearchIndexLedger, identifiers: &[&str], completion: impl FnOnce(Result<(), CSIndexError>)) {
        let r = ledger.delete_identifiers(identifiers).map(|ops| self.send_ops(ctx, ops));
        completion(r);
    }

    /// `deleteAllSearchableItems(completionHandler:)`.
    pub fn delete_all_searchable_items_with_completion(&self, ctx: &Ctx, ledger: &mut SearchIndexLedger, completion: impl FnOnce(Result<(), CSIndexError>)) {
        let r = ledger.delete_all().map(|ops| self.send_ops(ctx, ops));
        completion(r);
    }

    /// `endBatch(withClientState:completionHandler:)`: sends the held-back calls only when the whole batch is valid.
    pub fn end_batch_with_client_state(&self, ctx: &Ctx, ledger: &mut SearchIndexLedger, client_state: &[u8], completion: impl FnOnce(Result<(), CSIndexError>)) {
        let r = ledger.end_batch(client_state).map(|ops| self.send_ops(ctx, ops));
        completion(r);
    }

    /// `fetchLastClientState(completionHandler:)`.
    pub fn fetch_last_client_state(&self, ledger: &SearchIndexLedger, completion: impl FnOnce(Result<Vec<u8>, CSIndexError>)) {
        completion(ledger.fetch_last_client_state());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quicksearch::CSSearchableItemAttributeSet;

    fn item(id: &str) -> CSSearchableItem {
        CSSearchableItem::new(id, "notes", CSSearchableItemAttributeSet::new("Milk"))
    }

    fn code(e: CSIndexError) -> CSIndexErrorCode {
        e.code
    }

    #[test]
    fn index_reports_ok_and_returns_the_op_to_send() {
        let mut l = SearchIndexLedger::new();
        let ops = l.index(&[item("a")]).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(l.contains("a"));
    }

    #[test]
    fn an_empty_identifier_or_oversized_item_is_invalid_item_error_and_nothing_is_indexed() {
        let mut l = SearchIndexLedger::new();
        let big = CSSearchableItem::new("big", "d", CSSearchableItemAttributeSet::new("t").content_description(&"x".repeat(MAX_ATTRIBUTE_BYTES + 1)));
        let e = l.index(&[item("ok"), big]).unwrap_err();
        assert_eq!((e.code, e.item_position), (CSIndexErrorCode::InvalidItemError, Some(1)));
        assert_eq!(l.item_count(), 0);
        assert_eq!(code(l.index(&[item("")]).unwrap_err()), CSIndexErrorCode::InvalidItemError);
    }

    #[test]
    fn the_item_cap_is_quota_exceeded_not_eviction() {
        let mut l = SearchIndexLedger::new();
        let all: Vec<_> = (0..MAX_INDEXED_ITEMS).map(|i| item(&format!("n{i}"))).collect();
        l.index(&all).unwrap();
        assert_eq!(code(l.index(&[item("extra")]).unwrap_err()), CSIndexErrorCode::QuotaExceeded);
        assert_eq!(l.item_count(), MAX_INDEXED_ITEMS);
        assert!(l.contains("n0"));
        l.index(&[item("n0")]).unwrap();
    }

    #[test]
    fn a_batch_commits_all_and_stores_the_client_state() {
        let mut l = SearchIndexLedger::new();
        l.begin_batch().unwrap();
        assert!(l.index(&[item("a")]).unwrap().is_empty());
        assert!(l.delete_identifiers(&["a"]).unwrap().is_empty());
        l.index(&[item("b")]).unwrap();
        let ops = l.end_batch(b"v1").unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(l.fetch_last_client_state().unwrap(), b"v1");
        assert!(l.contains("b") && !l.contains("a"));
    }

    #[test]
    fn a_batch_with_a_failed_call_commits_none() {
        let mut l = SearchIndexLedger::new();
        l.index(&[item("keep")]).unwrap();
        l.begin_batch().unwrap();
        l.index(&[item("a")]).unwrap();
        l.delete_all().unwrap();
        assert!(l.index(&[item("")]).is_err());
        let e = l.end_batch(b"v2").unwrap_err();
        assert_eq!(e.code, CSIndexErrorCode::InvalidItemError);
        assert!(l.contains("keep") && !l.contains("a"));
        assert!(l.fetch_last_client_state().unwrap().is_empty());
    }

    #[test]
    fn client_state_errors() {
        let mut l = SearchIndexLedger::new();
        assert_eq!(code(l.end_batch(b"x").unwrap_err()), CSIndexErrorCode::InvalidClientStateError);
        l.begin_batch().unwrap();
        assert_eq!(code(l.begin_batch().unwrap_err()), CSIndexErrorCode::InvalidClientStateError);
        l.index(&[item("a")]).unwrap();
        assert_eq!(code(l.end_batch(&[0; MAX_CLIENT_STATE_BYTES + 1]).unwrap_err()), CSIndexErrorCode::InvalidClientStateError);
        assert!(!l.contains("a"));
    }

    #[test]
    fn a_closed_index_is_unavailable() {
        let mut l = SearchIndexLedger::new();
        l.close();
        assert_eq!(code(l.index(&[item("a")]).unwrap_err()), CSIndexErrorCode::IndexUnavailableError);
        assert_eq!(code(l.fetch_last_client_state().unwrap_err()), CSIndexErrorCode::IndexUnavailableError);
    }

    #[test]
    fn raw_values_are_stable() {
        assert_eq!(CSIndexErrorCode::QuotaExceeded.raw_value(), -1004);
        assert_eq!(CSIndexError::DOMAIN, "bifrost.quicksearch.index");
    }
}
