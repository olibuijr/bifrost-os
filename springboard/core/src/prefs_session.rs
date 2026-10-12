//! A system-wide preferences session: staged edits to one shared key-value store, committed under a lock and then
//! applied to the running system (std only).
//!
//! This is a pure model. It writes no file and signals no daemon. A caller opens a [`PrefsSession`] over the values
//! it loaded, edits them, and takes the lock before it commits. `commit` reports the keys that changed. `apply`
//! reports, once, the keys the caller must now push to the running system.
//!
//! Rules of this model:
//! - A key is one or more segments of ASCII letters, digits, `-` or `_`, joined by `.`. It is 1 to 128 bytes.
//! - A text value is at most 4096 bytes.
//! - Reads and edits work on the working copy, so a value set in the session is visible before it is committed.
//! - Only the lock holder may commit. The lock is exclusive: a second `lock` fails until `unlock`.
//! - `commit` moves the working copy into the committed copy and returns the keys whose value differs from it.
//! - `apply` returns the keys committed since the last `apply`, sorted, and clears them.
//! - `revert` drops uncommitted edits and restores the committed copy.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 128;

/// The longest text value, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;

/// A stored preference value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// Why a preferences call is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefsError {
    /// The key is empty, too long, or has an empty segment or a character outside the allowed set.
    InvalidKey(String),
    /// The text value is longer than [`MAX_TEXT_BYTES`] bytes.
    TextTooLong(usize),
    /// The lock is already held by this session.
    AlreadyLocked,
    /// The caller tried to commit or unlock without holding the lock.
    NotLocked,
}

impl fmt::Display for PrefsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrefsError::InvalidKey(k) => write!(f, "{k:?} is not a valid preference key"),
            PrefsError::TextTooLong(n) => {
                write!(f, "the text value is {n} bytes, the limit is {MAX_TEXT_BYTES}")
            }
            PrefsError::AlreadyLocked => write!(f, "the preferences are already locked"),
            PrefsError::NotLocked => write!(f, "the preferences must be locked before a commit"),
        }
    }
}

/// Checks a key against the rules of this module.
pub fn validate_key(key: &str) -> Result<(), PrefsError> {
    let bad = || PrefsError::InvalidKey(key.to_string());
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(bad());
    }
    for segment in key.split('.') {
        if segment.is_empty() {
            return Err(bad());
        }
        if !segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(bad());
        }
    }
    Ok(())
}

/// One open preferences session.
#[derive(Clone, Debug, Default)]
pub struct PrefsSession {
    committed: BTreeMap<String, PrefValue>,
    working: BTreeMap<String, PrefValue>,
    locked: bool,
    pending_apply: BTreeSet<String>,
}

impl PrefsSession {
    /// Opens a session over the values already on disk.
    pub fn open(values: BTreeMap<String, PrefValue>) -> PrefsSession {
        PrefsSession { committed: values.clone(), working: values, locked: false, pending_apply: BTreeSet::new() }
    }

    /// The current value of a key, including edits not yet committed.
    pub fn get(&self, key: &str) -> Option<&PrefValue> {
        self.working.get(key)
    }

    /// Sets a key in the working copy.
    pub fn set(&mut self, key: &str, value: PrefValue) -> Result<(), PrefsError> {
        validate_key(key)?;
        if let PrefValue::Text(text) = &value {
            if text.len() > MAX_TEXT_BYTES {
                return Err(PrefsError::TextTooLong(text.len()));
            }
        }
        self.working.insert(key.to_string(), value);
        Ok(())
    }

    /// Removes a key from the working copy. Returns the value it had, if any.
    pub fn remove(&mut self, key: &str) -> Result<Option<PrefValue>, PrefsError> {
        validate_key(key)?;
        Ok(self.working.remove(key))
    }

    /// Takes the exclusive lock needed for a commit.
    pub fn lock(&mut self) -> Result<(), PrefsError> {
        if self.locked {
            return Err(PrefsError::AlreadyLocked);
        }
        self.locked = true;
        Ok(())
    }

    /// Releases the lock. Uncommitted edits stay in the working copy.
    pub fn unlock(&mut self) -> Result<(), PrefsError> {
        if !self.locked {
            return Err(PrefsError::NotLocked);
        }
        self.locked = false;
        Ok(())
    }

    /// Whether this session holds the lock.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Whether the working copy differs from the committed copy.
    pub fn has_uncommitted_changes(&self) -> bool {
        self.working != self.committed
    }

    /// Makes the working copy the committed copy and returns the keys that changed, sorted.
    ///
    /// Fails with [`PrefsError::NotLocked`] when the lock is not held. A commit with no changes returns an empty list.
    pub fn commit(&mut self) -> Result<Vec<String>, PrefsError> {
        if !self.locked {
            return Err(PrefsError::NotLocked);
        }
        let changed: BTreeSet<String> = self
            .committed
            .keys()
            .chain(self.working.keys())
            .filter(|k| self.committed.get(*k) != self.working.get(*k))
            .cloned()
            .collect();
        self.committed = self.working.clone();
        self.pending_apply.extend(changed.iter().cloned());
        Ok(changed.into_iter().collect())
    }

    /// Returns the keys committed since the last apply, sorted, and clears them.
    pub fn apply(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_apply).into_iter().collect()
    }

    /// Drops the uncommitted edits and restores the committed copy.
    pub fn revert(&mut self) {
        self.working = self.committed.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> PrefValue {
        PrefValue::Text(s.to_string())
    }

    #[test]
    fn keys_are_dotted_segments_of_safe_characters() {
        assert!(validate_key("network.computer-name").is_ok());
        assert!(validate_key("a_b.c1").is_ok());
        assert_eq!(validate_key(""), Err(PrefsError::InvalidKey(String::new())));
        assert!(validate_key("network..name").is_err());
        assert!(validate_key(".name").is_err());
        assert!(validate_key("name.").is_err());
        assert!(validate_key("bad key").is_err());
        assert!(validate_key("slash/key").is_err());
        let long = "a".repeat(MAX_KEY_BYTES + 1);
        assert!(validate_key(&long).is_err());
        assert!(validate_key(&"a".repeat(MAX_KEY_BYTES)).is_ok());
    }

    #[test]
    fn edits_are_visible_before_commit() {
        let mut s = PrefsSession::open(BTreeMap::new());
        s.set("display.scale", PrefValue::Int(2)).unwrap();
        assert_eq!(s.get("display.scale"), Some(&PrefValue::Int(2)));
        assert!(s.has_uncommitted_changes());
        s.revert();
        assert_eq!(s.get("display.scale"), None);
        assert!(!s.has_uncommitted_changes());
    }

    #[test]
    fn commit_needs_the_lock() {
        let mut s = PrefsSession::open(BTreeMap::new());
        s.set("host.name", text("desk")).unwrap();
        assert_eq!(s.commit(), Err(PrefsError::NotLocked));
        s.lock().unwrap();
        assert_eq!(s.lock(), Err(PrefsError::AlreadyLocked));
        assert_eq!(s.commit().unwrap(), vec!["host.name".to_string()]);
        s.unlock().unwrap();
        assert_eq!(s.unlock(), Err(PrefsError::NotLocked));
    }

    #[test]
    fn commit_reports_only_keys_that_changed() {
        let mut start = BTreeMap::new();
        start.insert("a.same".to_string(), PrefValue::Bool(true));
        start.insert("a.old".to_string(), PrefValue::Int(1));
        let mut s = PrefsSession::open(start);
        s.lock().unwrap();
        s.set("a.same", PrefValue::Bool(true)).unwrap();
        s.set("a.old", PrefValue::Int(2)).unwrap();
        s.set("a.new", text("x")).unwrap();
        assert_eq!(s.commit().unwrap(), vec!["a.new".to_string(), "a.old".to_string()]);
        assert!(!s.has_uncommitted_changes());
        assert_eq!(s.commit().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn removal_counts_as_a_change_and_apply_runs_once() {
        let mut start = BTreeMap::new();
        start.insert("vpn.on-demand".to_string(), PrefValue::Bool(false));
        start.insert("proxy.host".to_string(), text("10.0.0.1"));
        let mut s = PrefsSession::open(start);
        s.lock().unwrap();
        assert_eq!(s.remove("proxy.host").unwrap(), Some(text("10.0.0.1")));
        s.set("vpn.on-demand", PrefValue::Bool(true)).unwrap();
        assert_eq!(s.commit().unwrap(), vec!["proxy.host".to_string(), "vpn.on-demand".to_string()]);
        assert_eq!(s.apply(), vec!["proxy.host".to_string(), "vpn.on-demand".to_string()]);
        assert!(s.apply().is_empty());
    }

    #[test]
    fn text_over_the_limit_is_refused_and_leaves_the_value_alone() {
        let mut s = PrefsSession::open(BTreeMap::new());
        s.set("note.body", text("ok")).unwrap();
        let big = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(s.set("note.body", PrefValue::Text(big)), Err(PrefsError::TextTooLong(MAX_TEXT_BYTES + 1)));
        assert_eq!(s.get("note.body"), Some(&text("ok")));
        assert!(s.set("note.body", PrefValue::Text("x".repeat(MAX_TEXT_BYTES))).is_ok());
    }
}
