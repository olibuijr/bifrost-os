//! A preferences session: one system-wide configuration store with three copies of its values.
//!
//! The working copy takes every `set`, `add` and `remove`. `commit` copies the working copy to the committed copy,
//! which is the copy a caller saves to persistent storage. `apply` makes the committed copy the active configuration,
//! which is what the system reads. Until a value is committed and applied, nothing outside the session sees it.
//! `synchronize` throws away uncommitted edits and returns the working copy to the committed one.
//!
//! This is a pure model. It reads and writes no file, takes no lock and needs no authorization. The caller keeps the
//! session, saves the committed copy in its own settings store and loads it back on start.
//!
//! Rules of this model:
//! - A key is a path that starts with `/`, has one or more segments, and has no empty segment, so `/a//b` and `/a/`
//!   are refused. Each segment holds ASCII letters, digits, `-`, `_` or `.`. A key is at most 255 bytes.
//! - `set` creates or replaces a value. `add` only creates one and refuses a key that already holds a value.
//! - `commit` returns `true` and moves the generation on only when the working copy differs from the committed copy.
//! - `apply` returns `true` only when the active copy changes. It fires [`PrefsEvent::Applied`] only then, and fires
//!   [`PrefsEvent::Committed`] from `commit` only when the committed copy changes.
//! - The generation starts at 0 and counts commits that changed the committed copy. [`PrefsSession::signature`] returns
//!   it, so a caller can tell whether saved configuration changed since it last looked.

use std::collections::BTreeMap;
use std::fmt;

/// The longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 255;

/// One stored value. Preferences here are booleans, whole numbers and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// What a session tells its callback. The number is the generation the event belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefsEvent {
    Committed(u64),
    Applied(u64),
}

/// Why a change is refused. A refused change leaves the session as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefsError {
    /// The key is not a valid path (see the rules above).
    InvalidKey(String),
    /// `add` was called for a key that already holds a value.
    KeyExists(String),
}

impl fmt::Display for PrefsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrefsError::InvalidKey(k) => write!(f, "{k:?} is not a valid preference key"),
            PrefsError::KeyExists(k) => write!(f, "a preference already exists at {k:?}"),
        }
    }
}

/// Checks a key against the rules of this model.
pub fn check_key(key: &str) -> Result<(), PrefsError> {
    let bad = || PrefsError::InvalidKey(key.to_string());
    if key.len() > MAX_KEY_BYTES {
        return Err(bad());
    }
    let rest = key.strip_prefix('/').ok_or_else(bad)?;
    if rest.is_empty() {
        return Err(bad());
    }
    for seg in rest.split('/') {
        let ok = !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !ok {
            return Err(bad());
        }
    }
    Ok(())
}

type PrefsCallback = Box<dyn FnMut(&PrefsEvent)>;

/// A preferences session. See the module comment for the rules.
pub struct PrefsSession {
    name: String,
    working: BTreeMap<String, PrefValue>,
    committed: BTreeMap<String, PrefValue>,
    applied: BTreeMap<String, PrefValue>,
    generation: u64,
    callback: Option<PrefsCallback>,
}

impl PrefsSession {
    /// Opens a session for the named store. The working, committed and active copies start empty.
    pub fn new(name: &str) -> PrefsSession {
        PrefsSession {
            name: name.to_string(),
            working: BTreeMap::new(),
            committed: BTreeMap::new(),
            applied: BTreeMap::new(),
            generation: 0,
            callback: None,
        }
    }

    /// The name the session was opened with.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The working value for a key. A key that is not valid holds nothing.
    pub fn get(&self, key: &str) -> Option<&PrefValue> {
        self.working.get(key)
    }

    /// The value a key holds in the active configuration.
    pub fn active(&self, key: &str) -> Option<&PrefValue> {
        self.applied.get(key)
    }

    /// Every key in the working copy, in sorted order.
    pub fn keys(&self) -> Vec<String> {
        self.working.keys().cloned().collect()
    }

    /// Creates or replaces the working value for a key.
    pub fn set(&mut self, key: &str, value: PrefValue) -> Result<(), PrefsError> {
        check_key(key)?;
        self.working.insert(key.to_string(), value);
        Ok(())
    }

    /// Creates the working value for a key. Refused when the key already holds a value.
    pub fn add(&mut self, key: &str, value: PrefValue) -> Result<(), PrefsError> {
        check_key(key)?;
        if self.working.contains_key(key) {
            return Err(PrefsError::KeyExists(key.to_string()));
        }
        self.working.insert(key.to_string(), value);
        Ok(())
    }

    /// Removes the working value for a key. Returns whether one was there.
    pub fn remove(&mut self, key: &str) -> Result<bool, PrefsError> {
        check_key(key)?;
        Ok(self.working.remove(key).is_some())
    }

    /// True when the working copy differs from the committed copy.
    pub fn has_uncommitted(&self) -> bool {
        self.working != self.committed
    }

    /// The generation of the committed copy. It moves on each commit that changed the committed copy.
    pub fn signature(&self) -> u64 {
        self.generation
    }

    /// Copies the working copy to the committed copy. Returns `true` when the committed copy changed.
    pub fn commit(&mut self) -> bool {
        if !self.has_uncommitted() {
            return false;
        }
        self.committed = self.working.clone();
        self.generation += 1;
        let event = PrefsEvent::Committed(self.generation);
        self.fire(event);
        true
    }

    /// Makes the committed copy the active configuration. Returns `true` when the active copy changed.
    pub fn apply(&mut self) -> bool {
        if self.applied == self.committed {
            return false;
        }
        self.applied = self.committed.clone();
        let event = PrefsEvent::Applied(self.generation);
        self.fire(event);
        true
    }

    /// Throws away uncommitted edits. Returns `true` when the working copy changed.
    pub fn synchronize(&mut self) -> bool {
        if !self.has_uncommitted() {
            return false;
        }
        self.working = self.committed.clone();
        true
    }

    /// Sets the function called on each commit and apply. It replaces any earlier one.
    pub fn set_callback<F>(&mut self, callback: F)
    where
        F: FnMut(&PrefsEvent) + 'static,
    {
        self.callback = Some(Box::new(callback));
    }

    fn fire(&mut self, event: PrefsEvent) {
        if let Some(cb) = self.callback.as_mut() {
            cb(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn text(s: &str) -> PrefValue {
        PrefValue::Text(s.to_string())
    }

    #[test]
    fn set_and_get_keep_keys_sorted() {
        let mut s = PrefsSession::new("system");
        s.set("/System/Name", text("desk")).unwrap();
        s.set("/Network/Global/Proxies/Enabled", PrefValue::Bool(true)).unwrap();
        s.set("/Network/Global/Proxies/Port", PrefValue::Int(8080)).unwrap();
        assert_eq!(s.get("/System/Name"), Some(&text("desk")));
        assert_eq!(
            s.keys(),
            vec![
                "/Network/Global/Proxies/Enabled".to_string(),
                "/Network/Global/Proxies/Port".to_string(),
                "/System/Name".to_string(),
            ]
        );
        // Replacing a value keeps one key.
        s.set("/System/Name", text("studio")).unwrap();
        assert_eq!(s.keys().len(), 3);
        assert_eq!(s.get("/System/Name"), Some(&text("studio")));
    }

    #[test]
    fn invalid_keys_are_refused() {
        let mut s = PrefsSession::new("system");
        for bad in ["system", "/", "/a//b", "/a/", "/a b", "/a/é"] {
            assert_eq!(s.set(bad, PrefValue::Int(1)), Err(PrefsError::InvalidKey(bad.to_string())));
        }
        let long = format!("/{}", "a".repeat(MAX_KEY_BYTES));
        assert!(matches!(check_key(&long), Err(PrefsError::InvalidKey(_))));
        assert!(s.keys().is_empty());
    }

    #[test]
    fn add_refuses_an_existing_key() {
        let mut s = PrefsSession::new("system");
        s.add("/Clock/Format", text("24h")).unwrap();
        assert_eq!(
            s.add("/Clock/Format", text("12h")),
            Err(PrefsError::KeyExists("/Clock/Format".to_string()))
        );
        assert_eq!(s.get("/Clock/Format"), Some(&text("24h")));
    }

    #[test]
    fn remove_reports_whether_a_value_was_there() {
        let mut s = PrefsSession::new("system");
        s.set("/Power/Sleep", PrefValue::Int(30)).unwrap();
        assert_eq!(s.remove("/Power/Sleep"), Ok(true));
        assert_eq!(s.remove("/Power/Sleep"), Ok(false));
        assert!(s.get("/Power/Sleep").is_none());
    }

    #[test]
    fn commit_moves_the_signature_only_on_change() {
        let mut s = PrefsSession::new("system");
        assert_eq!(s.signature(), 0);
        assert!(!s.commit(), "nothing to commit yet");
        assert_eq!(s.signature(), 0);
        s.set("/Display/Scale", PrefValue::Int(2)).unwrap();
        assert!(s.has_uncommitted());
        assert!(s.commit());
        assert_eq!(s.signature(), 1);
        assert!(!s.has_uncommitted());
        assert!(!s.commit());
        assert_eq!(s.signature(), 1);
    }

    #[test]
    fn apply_makes_only_committed_values_active() {
        let mut s = PrefsSession::new("system");
        s.set("/Display/Scale", PrefValue::Int(2)).unwrap();
        // Working value is not active before commit and apply.
        assert!(s.active("/Display/Scale").is_none());
        assert!(!s.apply(), "nothing committed, nothing to apply");
        s.commit();
        assert!(s.active("/Display/Scale").is_none(), "commit alone does not change the active copy");
        assert!(s.apply());
        assert_eq!(s.active("/Display/Scale"), Some(&PrefValue::Int(2)));
        assert!(!s.apply());
    }

    #[test]
    fn synchronize_discards_uncommitted_edits() {
        let mut s = PrefsSession::new("system");
        s.set("/Input/Speed", PrefValue::Int(5)).unwrap();
        s.commit();
        s.set("/Input/Speed", PrefValue::Int(9)).unwrap();
        s.set("/Input/Accel", PrefValue::Bool(false)).unwrap();
        assert!(s.synchronize());
        assert_eq!(s.get("/Input/Speed"), Some(&PrefValue::Int(5)));
        assert!(s.get("/Input/Accel").is_none());
        assert!(!s.has_uncommitted());
        assert!(!s.synchronize());
    }

    #[test]
    fn callback_sees_commit_then_apply_in_order() {
        let seen: Rc<RefCell<Vec<PrefsEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut s = PrefsSession::new("system");
        s.set_callback(move |e| sink.borrow_mut().push(*e));
        s.set("/Sound/Mute", PrefValue::Bool(true)).unwrap();
        s.commit();
        s.apply();
        s.apply(); // no change, no event
        s.commit(); // no change, no event
        assert_eq!(*seen.borrow(), vec![PrefsEvent::Committed(1), PrefsEvent::Applied(1)]);
    }
}
