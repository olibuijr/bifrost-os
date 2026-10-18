//! Document revisions (`GenerationalStorage` parity): a per-document history of saved versions, newest first, with
//! browse, restore and pruning. The reference keeps these in a revision daemon; BifrOSt keeps them in the document
//! controller's own store, so this module is the model and does no file I/O. The caller saves the bytes it wrote
//! (`RevisionStore::record`) when a document is saved, lists them for the Versions browser (`list`), and writes the
//! bytes of a chosen revision back (`restore`).
//!
//! Rules:
//! - A revision is kept only when its bytes differ from the newest one, so saving unchanged content adds nothing.
//! - At most `limit` revisions are kept per document. The oldest are pruned first.
//! - Restoring a revision does not rewrite history: the restored bytes become a new newest revision.

use std::collections::BTreeMap;

/// The number of revisions kept per document unless the store is built with another limit.
pub const DEFAULT_LIMIT: usize = 50;

/// One saved version of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Monotonic per store, so it orders revisions even when the clock repeats.
    pub seq: u64,
    /// Seconds since the epoch, as the caller reports them.
    pub saved_at: i64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RevisionStore {
    limit: usize,
    next_seq: u64,
    docs: BTreeMap<String, Vec<Revision>>,
}

impl Default for RevisionStore {
    fn default() -> Self {
        Self::with_limit(DEFAULT_LIMIT)
    }
}

impl RevisionStore {
    /// A store that keeps at most `limit` revisions per document. A limit of zero is raised to one.
    pub fn with_limit(limit: usize) -> Self {
        RevisionStore { limit: limit.max(1), next_seq: 1, docs: BTreeMap::new() }
    }

    /// Record a saved version. Returns the new revision, or `None` when the bytes equal the newest revision.
    pub fn record(&mut self, doc: &str, saved_at: i64, bytes: &[u8]) -> Option<Revision> {
        let history = self.docs.entry(doc.to_string()).or_default();
        if history.last().is_some_and(|newest| newest.bytes == bytes) {
            return None;
        }
        let rev = Revision { seq: self.next_seq, saved_at, bytes: bytes.to_vec() };
        self.next_seq += 1;
        history.push(rev.clone());
        if history.len() > self.limit {
            let extra = history.len() - self.limit;
            history.drain(..extra);
        }
        Some(rev)
    }

    /// The revisions of a document, newest first. Empty for an unknown document.
    pub fn list(&self, doc: &str) -> Vec<&Revision> {
        self.docs.get(doc).map(|h| h.iter().rev().collect()).unwrap_or_default()
    }

    /// The bytes of the revision with this sequence number.
    pub fn get(&self, doc: &str, seq: u64) -> Option<&Revision> {
        self.docs.get(doc)?.iter().find(|r| r.seq == seq)
    }

    /// The bytes to write back for a chosen revision. The restored content is recorded as the newest revision,
    /// so the history keeps the version that was replaced. Returns the bytes restored, or `None` when the
    /// revision is unknown.
    pub fn restore(&mut self, doc: &str, seq: u64, saved_at: i64) -> Option<Vec<u8>> {
        let bytes = self.get(doc, seq)?.bytes.clone();
        self.record(doc, saved_at, &bytes);
        Some(bytes)
    }

    /// Forget a document's history, for example when the document is deleted.
    pub fn remove(&mut self, doc: &str) -> bool {
        self.docs.remove(doc).is_some()
    }

    /// Number of documents with a history.
    pub fn document_count(&self) -> usize {
        self.docs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_unchanged_bytes_adds_no_revision() {
        let mut s = RevisionStore::default();
        assert!(s.record("a.txt", 10, b"one").is_some());
        assert!(s.record("a.txt", 11, b"one").is_none());
        assert_eq!(s.list("a.txt").len(), 1);
    }

    #[test]
    fn the_list_is_newest_first_and_per_document() {
        let mut s = RevisionStore::default();
        s.record("a.txt", 10, b"one");
        s.record("a.txt", 20, b"two");
        s.record("b.txt", 30, b"other");
        let seqs: Vec<(u64, i64)> = s.list("a.txt").iter().map(|r| (r.seq, r.saved_at)).collect();
        assert_eq!(seqs, vec![(2, 20), (1, 10)]);
        assert_eq!(s.list("b.txt").len(), 1);
        assert!(s.list("missing").is_empty());
        assert_eq!(s.document_count(), 2);
    }

    #[test]
    fn the_oldest_revisions_are_pruned_at_the_limit() {
        let mut s = RevisionStore::with_limit(2);
        s.record("a", 1, b"1");
        s.record("a", 2, b"2");
        s.record("a", 3, b"3");
        let kept: Vec<&[u8]> = s.list("a").iter().map(|r| r.bytes.as_slice()).collect();
        assert_eq!(kept, vec![b"3".as_slice(), b"2".as_slice()]);
        assert!(s.get("a", 1).is_none());
    }

    #[test]
    fn a_zero_limit_keeps_the_newest_revision() {
        let mut s = RevisionStore::with_limit(0);
        s.record("a", 1, b"1");
        s.record("a", 2, b"2");
        assert_eq!(s.list("a").len(), 1);
        assert_eq!(s.list("a")[0].bytes, b"2".to_vec());
    }

    #[test]
    fn restore_returns_the_old_bytes_and_keeps_the_replaced_version() {
        let mut s = RevisionStore::default();
        s.record("a", 1, b"first");
        s.record("a", 2, b"second");
        let restored = s.restore("a", 1, 3).expect("revision 1 exists");
        assert_eq!(restored, b"first".to_vec());
        let list = s.list("a");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].bytes, b"first".to_vec());
        assert_eq!(list[1].bytes, b"second".to_vec());
        assert!(s.restore("a", 99, 4).is_none());
    }

    #[test]
    fn remove_drops_a_document_history() {
        let mut s = RevisionStore::default();
        s.record("a", 1, b"x");
        assert!(s.remove("a"));
        assert!(!s.remove("a"));
        assert!(s.list("a").is_empty());
    }
}
