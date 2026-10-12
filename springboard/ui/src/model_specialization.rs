//! A cache of specialized model artifacts, keyed by model name and specialization options.
//! Preparing a model for this device is slow, so the result is kept and reused for the same model and options.
//! Each entry has a [`Policy`]: `Purgeable` entries share a cost-limited least-recently-used cache (`cache::Cache`)
//! and go first, by the limit or by [`SpecializationCache::purge`]; `Persistent` entries are never evicted by the limit
//! or by a purge, and leave only through an explicit delete. The artifact is opaque bytes supplied by the caller.
//! Nothing here writes to disk.
use crate::cache::Cache;
use std::collections::{BTreeMap, BTreeSet};

/// The identity of one entry: the model name and the specialization options it was prepared with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    model: String,
    options: String,
}

impl Key {
    /// The key for a model name and its options text.
    pub fn new(model: &str, options: &str) -> Self {
        Key { model: model.to_owned(), options: options.to_owned() }
    }

    /// The model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The specialization options text.
    pub fn options(&self) -> &str {
        &self.options
    }
}

/// When the system may drop an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Dropped by the cost limit (least recently used first) or by [`SpecializationCache::purge`].
    Purgeable,
    /// Kept until it is deleted explicitly, whatever the limit or a purge does.
    Persistent,
}

/// The specialized artifacts of every model the app has prepared.
pub struct SpecializationCache {
    purgeable: Cache<Key, Vec<u8>>,
    // Mirrors the keys held by `purgeable`, so that a model's entries can be found without scanning the LRU.
    purgeable_keys: BTreeSet<Key>,
    persistent: BTreeMap<Key, Vec<u8>>,
}

impl SpecializationCache {
    /// A cache whose purgeable entries together may cost at most `purgeable_limit` bytes.
    pub fn new(purgeable_limit: usize) -> Self {
        SpecializationCache {
            purgeable: Cache::new(purgeable_limit),
            purgeable_keys: BTreeSet::new(),
            persistent: BTreeMap::new(),
        }
    }

    /// Stores an artifact, replacing any entry with the same model and options whatever its policy was.
    /// A purgeable artifact larger than the limit is not kept.
    pub fn store(&mut self, model: &str, options: &str, artifact: Vec<u8>, policy: Policy) {
        self.delete_entry(model, options);
        let key = Key::new(model, options);
        match policy {
            Policy::Persistent => {
                self.persistent.insert(key, artifact);
            }
            Policy::Purgeable => {
                let cost = artifact.len();
                self.purgeable.put(key.clone(), artifact, cost);
                self.purgeable_keys.insert(key);
                // The limit may have evicted older entries, and an oversize artifact was never kept.
                let held = &self.purgeable;
                self.purgeable_keys.retain(|k| held.contains_key(k));
            }
        }
    }

    /// The artifact for a model and its options, if held. A purgeable entry is marked as recently used.
    pub fn lookup(&mut self, model: &str, options: &str) -> Option<&[u8]> {
        let key = Key::new(model, options);
        if let Some(bytes) = self.purgeable.get(&key) {
            return Some(bytes.as_slice());
        }
        self.persistent.get(&key).map(|bytes| bytes.as_slice())
    }

    /// The policy of a held entry, or `None` when nothing is stored for the model and options.
    pub fn policy_of(&self, model: &str, options: &str) -> Option<Policy> {
        let key = Key::new(model, options);
        if self.persistent.contains_key(&key) {
            Some(Policy::Persistent)
        } else if self.purgeable.contains_key(&key) {
            Some(Policy::Purgeable)
        } else {
            None
        }
    }

    /// Deletes the entry for one model and its options. Returns whether there was one.
    pub fn delete_entry(&mut self, model: &str, options: &str) -> bool {
        let key = Key::new(model, options);
        let purged = self.purgeable.remove(&key).is_some();
        self.purgeable_keys.remove(&key);
        let kept = self.persistent.remove(&key).is_some();
        purged || kept
    }

    /// Deletes every entry of one model, whatever its options and policy. Returns how many went.
    pub fn delete_entries_for(&mut self, model: &str) -> usize {
        let mut keys: Vec<Key> = self.purgeable_keys.iter().filter(|k| k.model == model).cloned().collect();
        keys.extend(self.persistent.keys().filter(|k| k.model == model).cloned());
        for key in &keys {
            self.purgeable.remove(key);
            self.purgeable_keys.remove(key);
            self.persistent.remove(key);
        }
        keys.len()
    }

    /// Drops every purgeable entry, as under memory pressure. Persistent entries stay. Returns how many went.
    pub fn purge(&mut self) -> usize {
        let keys: Vec<Key> = self.purgeable_keys.iter().cloned().collect();
        for key in &keys {
            self.purgeable.remove(key);
        }
        self.purgeable_keys.clear();
        keys.len()
    }

    /// Deletes every entry, both policies.
    pub fn delete_all(&mut self) {
        self.purge();
        self.persistent.clear();
    }

    /// The number of held entries.
    pub fn len(&self) -> usize {
        self.purgeable.len() + self.persistent.len()
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes held by both policies together.
    pub fn total_cost(&self) -> usize {
        self.purgeable.total_cost() + self.persistent.values().map(|bytes| bytes.len()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_finds_by_model_and_options() {
        let mut c = SpecializationCache::new(100);
        c.store("tagger", "gpu", vec![1, 2, 3], Policy::Purgeable);
        c.store("tagger", "cpu", vec![9], Policy::Purgeable);
        assert_eq!(c.lookup("tagger", "gpu"), Some(&[1u8, 2, 3][..]));
        assert_eq!(c.lookup("tagger", "cpu"), Some(&[9u8][..]));
        assert_eq!(c.lookup("tagger", "other"), None);
        assert_eq!((c.len(), c.total_cost()), (2, 4));
    }

    #[test]
    fn purgeable_entries_go_least_recently_used_first() {
        let mut c = SpecializationCache::new(10);
        c.store("a", "", vec![0; 4], Policy::Purgeable);
        c.store("b", "", vec![0; 4], Policy::Purgeable);
        assert!(c.lookup("a", "").is_some()); // a is now newer than b
        c.store("c", "", vec![0; 4], Policy::Purgeable); // 12 > 10: b goes
        assert!(c.lookup("b", "").is_none());
        assert!(c.lookup("a", "").is_some() && c.lookup("c", "").is_some());
        assert_eq!(c.total_cost(), 8);
    }

    #[test]
    fn persistent_entries_survive_the_limit_and_a_purge() {
        let mut c = SpecializationCache::new(4);
        c.store("p", "", vec![7; 3], Policy::Persistent);
        c.store("q", "", vec![1; 4], Policy::Purgeable);
        c.store("r", "", vec![2; 4], Policy::Purgeable); // 8 > 4: q goes, the persistent 3 bytes do not count
        assert!(c.lookup("q", "").is_none());
        assert_eq!(c.policy_of("p", ""), Some(Policy::Persistent));
        assert_eq!(c.purge(), 1); // only r was purgeable
        assert!(c.lookup("p", "").is_some());
        assert_eq!((c.len(), c.total_cost()), (1, 3));
    }

    #[test]
    fn delete_entries_for_removes_one_model_only() {
        let mut c = SpecializationCache::new(100);
        c.store("m", "cpu", vec![1], Policy::Purgeable);
        c.store("m", "gpu", vec![2, 2], Policy::Persistent);
        c.store("n", "cpu", vec![3], Policy::Purgeable);
        assert_eq!(c.delete_entries_for("m"), 2);
        assert!(c.lookup("m", "cpu").is_none() && c.lookup("m", "gpu").is_none());
        assert!(c.lookup("n", "cpu").is_some());
        assert_eq!((c.len(), c.total_cost()), (1, 1));
    }

    #[test]
    fn oversize_purgeable_artifact_is_not_kept_and_not_indexed() {
        let mut c = SpecializationCache::new(5);
        c.store("big", "", vec![0; 6], Policy::Purgeable);
        assert_eq!((c.lookup("big", "").is_none(), c.policy_of("big", ""), c.len()), (true, None, 0));
        c.store("s", "", vec![0; 5], Policy::Purgeable);
        // A stale index entry for "big" would make this count 2.
        assert_eq!(c.purge(), 1);
        assert!(c.is_empty());
    }

    #[test]
    fn persistent_store_replaces_a_purgeable_entry_without_double_counting() {
        let mut c = SpecializationCache::new(100);
        c.store("m", "", vec![1; 4], Policy::Purgeable);
        assert_eq!(c.total_cost(), 4);
        c.store("m", "", vec![2; 2], Policy::Persistent);
        assert_eq!(c.policy_of("m", ""), Some(Policy::Persistent));
        assert_eq!((c.len(), c.total_cost()), (1, 2));
        assert_eq!(c.purge(), 0);
        assert_eq!(c.lookup("m", ""), Some(&[2u8, 2][..]));
    }

    #[test]
    fn delete_all_clears_both_policies() {
        let mut c = SpecializationCache::new(100);
        c.store("x", "", vec![1; 2], Policy::Purgeable);
        c.store("y", "", vec![1; 3], Policy::Persistent);
        c.delete_all();
        assert_eq!((c.len(), c.total_cost(), c.delete_entry("x", "")), (0, 0, false));
    }
}
