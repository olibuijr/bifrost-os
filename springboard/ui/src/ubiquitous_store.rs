//! Small key-value store that keeps a set of settings in step across a person's devices.
//!
//! `UbiquitousStore` is the local replica of one device. Writes go to the local map at once and are queued as
//! pending; `synchronize` drains that queue into `Change` records for the transport to upload. Changes from other
//! devices come in through `apply_remote` or `apply_initial`. Each key carries a logical stamp and the name of the
//! device that wrote it. A remote change wins only when its (stamp, origin) pair is greater than the local one, so
//! the newest write wins and a duplicate change is a no-op. The local clock moves forward past every remote stamp,
//! so a later local write always sorts after the writes it has seen.
//!
//! Removals keep a tombstone (a `Change` with no value) so the removal can win over an older write from another
//! device. Two limits apply to the live data: a total byte budget (key length plus value size, summed over live
//! keys) and a maximum number of live keys. A write that would break either limit is refused, and the store records
//! `ChangeReason::QuotaViolationChange`. `take_changes` returns the reason and the sorted keys changed by remote
//! data since the last call, which is what a change observer needs.
//!
//! Gaps: no transport, no account tracking (`account_changed` only records the reason), no nested arrays or
//! dictionaries (values are scalars and byte blobs), no per-device cursors, and the quota numbers are chosen by the
//! caller instead of taken from the system.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A value a key can hold.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Double(f64),
    Int(i64),
    Text(String),
    Data(Vec<u8>),
}

impl Value {
    /// The number of bytes this value counts against the quota. Scalars count 1 byte for a bool and 8 for a
    /// double or an integer; text and data count their length.
    pub fn size_bytes(&self) -> usize {
        match self {
            Value::Bool(_) => 1,
            Value::Double(_) | Value::Int(_) => 8,
            Value::Text(s) => s.len(),
            Value::Data(d) => d.len(),
        }
    }
}

/// Why the store changed, as reported by `take_changes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeReason {
    ServerChange,
    InitialSyncChange,
    QuotaViolationChange,
    AccountChange,
}

/// One key's state as it moves between devices. `value: None` is a removal.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub key: String,
    pub value: Option<Value>,
    pub stamp: u64,
    pub origin: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    EmptyKey,
    QuotaExceeded,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::EmptyKey => write!(f, "the key is empty"),
            StoreError::QuotaExceeded => write!(f, "the write would exceed the store quota"),
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    value: Option<Value>,
    stamp: u64,
    origin: String,
}

/// The local replica of a key-value store on one device.
#[derive(Debug)]
pub struct UbiquitousStore {
    device: String,
    clock: u64,
    quota_bytes: usize,
    max_keys: usize,
    entries: BTreeMap<String, Entry>,
    pending: BTreeSet<String>,
    changed: BTreeSet<String>,
    reason: Option<ChangeReason>,
}

impl UbiquitousStore {
    /// An empty replica for the device `device`, with the given byte budget and live-key limit.
    pub fn new(device: &str, quota_bytes: usize, max_keys: usize) -> Self {
        UbiquitousStore {
            device: device.to_string(),
            clock: 0,
            quota_bytes,
            max_keys,
            entries: BTreeMap::new(),
            pending: BTreeSet::new(),
            changed: BTreeSet::new(),
            reason: None,
        }
    }

    /// The live value for `key`, or `None` when the key is missing or removed.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key).and_then(|e| e.value.as_ref())
    }

    /// The live keys in sorted order.
    pub fn keys(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(_, e)| e.value.is_some())
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// The number of live keys.
    pub fn len(&self) -> usize {
        self.entries.values().filter(|e| e.value.is_some()).count()
    }

    /// Whether the store holds no live keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes used by live keys: key length plus value size, summed.
    pub fn size_bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|(k, e)| e.value.as_ref().map(|v| k.len() + v.size_bytes()))
            .sum()
    }

    /// Writes `value` under `key`. The write is refused and nothing changes when the key is empty or the quota
    /// would be exceeded. A refused write for quota reasons sets `QuotaViolationChange`.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), StoreError> {
        if key.is_empty() {
            return Err(StoreError::EmptyKey);
        }
        if !self.fits(key, Some(&value)) {
            self.reason = Some(ChangeReason::QuotaViolationChange);
            return Err(StoreError::QuotaExceeded);
        }
        self.clock += 1;
        self.entries.insert(
            key.to_string(),
            Entry {
                value: Some(value),
                stamp: self.clock,
                origin: self.device.clone(),
            },
        );
        self.pending.insert(key.to_string());
        Ok(())
    }

    /// Removes `key`. A key that is not live is left alone and nothing is queued.
    pub fn remove(&mut self, key: &str) {
        if self.get(key).is_none() {
            return;
        }
        self.clock += 1;
        self.entries.insert(
            key.to_string(),
            Entry {
                value: None,
                stamp: self.clock,
                origin: self.device.clone(),
            },
        );
        self.pending.insert(key.to_string());
    }

    /// Drains the queue of local writes and removals, sorted by key. Each change carries the stamp it was written
    /// with, so the transport can send it unchanged.
    pub fn synchronize(&mut self) -> Vec<Change> {
        let keys: Vec<String> = std::mem::take(&mut self.pending).into_iter().collect();
        keys.into_iter()
            .filter_map(|k| {
                let e = self.entries.get(&k)?;
                Some(Change {
                    key: k.clone(),
                    value: e.value.clone(),
                    stamp: e.stamp,
                    origin: e.origin.clone(),
                })
            })
            .collect()
    }

    /// Applies one change from another device. Returns `true` when it was accepted. A change is rejected when the
    /// local entry is at least as new, or when accepting it would break the quota (a rejected change for quota
    /// reasons sets `QuotaViolationChange`). An accepted change also drops any local pending write for the key,
    /// since the other device's value is now the state.
    pub fn apply_remote(&mut self, change: Change) -> bool {
        if change.key.is_empty() {
            return false;
        }
        if let Some(cur) = self.entries.get(&change.key) {
            if (cur.stamp, cur.origin.as_str()) >= (change.stamp, change.origin.as_str()) {
                return false;
            }
        }
        if !self.fits(&change.key, change.value.as_ref()) {
            self.reason = Some(ChangeReason::QuotaViolationChange);
            return false;
        }
        self.clock = self.clock.max(change.stamp);
        self.pending.remove(&change.key);
        self.changed.insert(change.key.clone());
        if self.reason.is_none() {
            self.reason = Some(ChangeReason::ServerChange);
        }
        self.entries.insert(
            change.key,
            Entry {
                value: change.value,
                stamp: change.stamp,
                origin: change.origin,
            },
        );
        true
    }

    /// Applies the first snapshot from the other devices. Returns how many changes were accepted. The reason is
    /// `InitialSyncChange`, unless a later quota rejection replaces it.
    pub fn apply_initial(&mut self, changes: Vec<Change>) -> usize {
        self.reason = Some(ChangeReason::InitialSyncChange);
        let mut accepted = 0;
        for change in changes {
            if self.apply_remote(change) {
                accepted += 1;
            }
        }
        accepted
    }

    /// Records that the account behind the store changed.
    pub fn account_changed(&mut self) {
        self.reason = Some(ChangeReason::AccountChange);
    }

    /// Returns the pending reason and the sorted keys changed by remote data, then clears both. Returns `None`
    /// when there is no reason to report.
    pub fn take_changes(&mut self) -> Option<(ChangeReason, Vec<String>)> {
        let reason = self.reason.take()?;
        let keys: Vec<String> = std::mem::take(&mut self.changed).into_iter().collect();
        Some((reason, keys))
    }

    /// Whether a live value of `value` (or a removal, for `None`) fits under the quota and key limit once it
    /// replaces the current entry for `key`.
    fn fits(&self, key: &str, value: Option<&Value>) -> bool {
        let (old_size, was_live) = match self.entries.get(key).and_then(|e| e.value.as_ref()) {
            Some(v) => (key.len() + v.size_bytes(), true),
            None => (0, false),
        };
        let new_size = value.map_or(0, |v| key.len() + v.size_bytes());
        let total = self.size_bytes().saturating_sub(old_size).saturating_add(new_size);
        let count = self.len().saturating_sub(usize::from(was_live)) + usize::from(value.is_some());
        total <= self.quota_bytes && count <= self.max_keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    fn remote(key: &str, value: Option<Value>, stamp: u64, origin: &str) -> Change {
        Change {
            key: key.to_string(),
            value,
            stamp,
            origin: origin.to_string(),
        }
    }

    #[test]
    fn set_get_and_size_count_key_and_value() {
        let mut s = UbiquitousStore::new("laptop", 100, 10);
        s.set("theme", text("dark")).unwrap();
        // "theme" (5) + "dark" (4) = 9
        assert_eq!(s.size_bytes(), 9);
        assert_eq!(s.get("theme"), Some(&text("dark")));
        s.set("volume", Value::Double(0.5)).unwrap();
        // "volume" (6) + 8 = 14, total 23
        assert_eq!(s.size_bytes(), 23);
        assert_eq!(s.len(), 2);
        assert_eq!(s.keys(), vec!["theme", "volume"]);
        assert!(!s.is_empty());
    }

    #[test]
    fn quota_refuses_the_write_and_reports_the_reason() {
        let mut s = UbiquitousStore::new("laptop", 12, 10);
        s.set("a", Value::Int(1)).unwrap(); // 1 + 8 = 9
        s.set("bb", Value::Bool(true)).unwrap(); // 2 + 1, total 12, still within 12
        assert_eq!(s.set("c", Value::Bool(true)), Err(StoreError::QuotaExceeded)); // would be 14
        assert_eq!(s.get("c"), None);
        assert_eq!(s.size_bytes(), 12);
        assert_eq!(
            s.take_changes(),
            Some((ChangeReason::QuotaViolationChange, vec![]))
        );
        assert_eq!(s.take_changes(), None);
    }

    #[test]
    fn key_limit_allows_overwrite_but_not_a_new_key() {
        let mut s = UbiquitousStore::new("laptop", 1000, 1);
        s.set("a", Value::Bool(true)).unwrap();
        assert_eq!(s.set("b", Value::Bool(true)), Err(StoreError::QuotaExceeded));
        // Replacing the live key keeps the count at 1.
        assert_eq!(s.set("a", Value::Bool(false)), Ok(()));
        assert_eq!(s.set("", Value::Bool(false)), Err(StoreError::EmptyKey));
    }

    #[test]
    fn synchronize_drains_pending_writes_in_key_order() {
        let mut s = UbiquitousStore::new("laptop", 100, 10);
        s.set("x", Value::Int(7)).unwrap(); // stamp 1
        s.set("y", Value::Bool(false)).unwrap(); // stamp 2
        s.remove("x"); // stamp 3, tombstone
        s.remove("missing"); // not live, nothing queued, clock unchanged
        let out = s.synchronize();
        assert_eq!(
            out,
            vec![
                Change {
                    key: "x".to_string(),
                    value: None,
                    stamp: 3,
                    origin: "laptop".to_string()
                },
                Change {
                    key: "y".to_string(),
                    value: Some(Value::Bool(false)),
                    stamp: 2,
                    origin: "laptop".to_string()
                },
            ]
        );
        assert!(s.synchronize().is_empty());
    }

    #[test]
    fn newer_remote_change_wins_and_older_one_is_ignored() {
        let mut s = UbiquitousStore::new("phone", 100, 10);
        s.set("k", text("old")).unwrap(); // stamp 1, origin phone
        assert!(s.apply_remote(remote("k", Some(text("new")), 5, "laptop")));
        assert_eq!(s.get("k"), Some(&text("new")));
        assert!(!s.apply_remote(remote("k", Some(text("stale")), 3, "laptop")));
        assert_eq!(s.get("k"), Some(&text("new")));
        assert_eq!(
            s.take_changes(),
            Some((ChangeReason::ServerChange, vec!["k".to_string()]))
        );
    }

    #[test]
    fn equal_stamps_break_ties_by_origin_name() {
        let mut s = UbiquitousStore::new("laptop", 100, 10);
        s.set("k", Value::Int(1)).unwrap(); // (1, "laptop")
        // "phone" > "laptop", so the phone write wins.
        assert!(s.apply_remote(remote("k", Some(Value::Int(2)), 1, "phone")));
        assert_eq!(s.get("k"), Some(&Value::Int(2)));
        // (1, "phone") >= (1, "aaa"), so this is rejected.
        assert!(!s.apply_remote(remote("k", Some(Value::Int(3)), 1, "aaa")));
        assert_eq!(s.get("k"), Some(&Value::Int(2)));
    }

    #[test]
    fn remote_removal_wins_and_drops_the_local_pending_write() {
        let mut s = UbiquitousStore::new("laptop", 100, 10);
        s.set("k", Value::Bool(true)).unwrap(); // stamp 1, pending
        assert!(s.apply_remote(remote("k", None, 4, "phone")));
        assert_eq!(s.get("k"), None);
        assert!(s.keys().is_empty());
        assert!(s.synchronize().is_empty());
    }

    #[test]
    fn local_clock_moves_past_remote_stamps() {
        let mut s = UbiquitousStore::new("phone", 100, 10);
        assert!(s.apply_remote(remote("a", Some(Value::Int(1)), 10, "laptop")));
        s.set("b", Value::Int(2)).unwrap();
        let out = s.synchronize();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "b");
        assert_eq!(out[0].stamp, 11);
    }

    #[test]
    fn initial_sync_counts_accepted_and_reports_quota_rejection() {
        let mut s = UbiquitousStore::new("phone", 10, 10);
        let accepted = s.apply_initial(vec![
            remote("a", Some(Value::Int(1)), 1, "laptop"), // 1 + 8 = 9, fits
            remote("b", Some(Value::Bool(true)), 2, "laptop"), // 9 + 2 = 11, over 10
        ]);
        assert_eq!(accepted, 1);
        assert_eq!(s.get("a"), Some(&Value::Int(1)));
        assert_eq!(s.get("b"), None);
        // The later quota rejection replaces the initial-sync reason.
        assert_eq!(
            s.take_changes(),
            Some((ChangeReason::QuotaViolationChange, vec!["a".to_string()]))
        );
    }

    #[test]
    fn account_change_is_reported_with_no_keys() {
        let mut s = UbiquitousStore::new("phone", 10, 10);
        s.account_changed();
        assert_eq!(s.take_changes(), Some((ChangeReason::AccountChange, vec![])));
    }
}
