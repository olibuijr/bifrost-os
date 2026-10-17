//! System configuration store: the keyed, shared table of network and system state that apps read and that the
//! shell writes, the model behind `SCDynamicStore` (System Configuration framework). Keys are paths in two scopes,
//! as the framework names them: `State:/...` for what the system is doing now (addresses, the default route, the
//! Wi-Fi link) and `Setup:/...` for what the user configured (services, proxies, DNS servers).
//!
//! A key is `<scope>:/<component>/<component>...`. Components are non-empty, contain no whitespace, and are not `.` or
//! `..`. Values are strings, as the shell publishes them. Every real change bumps [`SystemConfigStore::revision`], so a
//! reader that remembers the last revision it saw knows whether anything changed; setting a key to the value it
//! already has changes nothing and does not bump it.

use std::collections::BTreeMap;

/// Why a key was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The key is not `State:/...` or `Setup:/...`, or one of its components is empty, dotted or contains whitespace.
    BadKey(String),
}

/// Checks a key and returns its scope (`State` or `Setup`).
pub fn validate_key(key: &str) -> Result<&str, StoreError> {
    let bad = || StoreError::BadKey(key.to_string());
    let (scope, path) = key.split_once(":/").ok_or_else(bad)?;
    if scope != "State" && scope != "Setup" {
        return Err(bad());
    }
    if path.is_empty() {
        return Err(bad());
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." || component.chars().any(char::is_whitespace) {
            return Err(bad());
        }
    }
    Ok(scope)
}

/// The shared table. Keys are kept sorted, so [`SystemConfigStore::keys`] lists them in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemConfigStore {
    values: BTreeMap<String, String>,
    revision: u64,
}

impl SystemConfigStore {
    /// An empty store at revision 0.
    pub fn new() -> SystemConfigStore {
        SystemConfigStore::default()
    }

    /// Sets `key` to `value`. Bumps the revision only when the stored value changes.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        validate_key(key)?;
        if self.values.get(key).map(String::as_str) != Some(value) {
            self.values.insert(key.to_string(), value.to_string());
            self.revision += 1;
        }
        Ok(())
    }

    /// The value at `key`, if any.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Removes `key`. Returns whether it was there; a removal bumps the revision.
    pub fn remove(&mut self, key: &str) -> bool {
        let removed = self.values.remove(key).is_some();
        if removed {
            self.revision += 1;
        }
        removed
    }

    /// Every key that starts with `prefix`, in order. An empty prefix lists them all.
    pub fn keys(&self, prefix: &str) -> Vec<&str> {
        self.values.keys().filter(|k| k.starts_with(prefix)).map(String::as_str).collect()
    }

    /// The number of real changes made so far.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_must_name_a_scope_and_clean_path() {
        assert_eq!(validate_key("State:/Network/Global/IPv4"), Ok("State"));
        assert_eq!(validate_key("Setup:/Network/Service/wifi0/Proxies"), Ok("Setup"));
        for bad in ["", "State", "Global:/Network", "State:/", "State:/Network//IPv4", "State:/Network/../IPv4", "State:/Net work", "State:Network"] {
            assert_eq!(validate_key(bad), Err(StoreError::BadKey(bad.to_string())), "{bad:?} must be refused");
        }
    }

    #[test]
    fn set_get_and_remove_round_trip() {
        let mut s = SystemConfigStore::new();
        assert_eq!(s.get("State:/Network/Global/IPv4"), None);
        s.set("State:/Network/Global/IPv4", "10.0.0.5").unwrap();
        assert_eq!(s.get("State:/Network/Global/IPv4"), Some("10.0.0.5"));
        assert!(s.remove("State:/Network/Global/IPv4"));
        assert!(!s.remove("State:/Network/Global/IPv4"), "a second removal finds nothing");
        assert_eq!(s.get("State:/Network/Global/IPv4"), None);
    }

    #[test]
    fn a_bad_key_is_refused_and_stores_nothing() {
        let mut s = SystemConfigStore::new();
        assert!(s.set("Oops:/x", "1").is_err());
        assert!(s.keys("").is_empty());
        assert_eq!(s.revision(), 0);
    }

    #[test]
    fn revision_moves_only_on_real_changes() {
        let mut s = SystemConfigStore::new();
        s.set("State:/Network/Global/DNS", "1.1.1.1").unwrap();
        assert_eq!(s.revision(), 1);
        s.set("State:/Network/Global/DNS", "1.1.1.1").unwrap();
        assert_eq!(s.revision(), 1, "the same value is not a change");
        s.set("State:/Network/Global/DNS", "9.9.9.9").unwrap();
        assert_eq!(s.revision(), 2);
        assert!(!s.remove("State:/Missing"));
        assert_eq!(s.revision(), 2, "removing a missing key is not a change");
        s.remove("State:/Network/Global/DNS");
        assert_eq!(s.revision(), 3);
    }

    #[test]
    fn keys_list_in_order_by_prefix() {
        let mut s = SystemConfigStore::new();
        s.set("Setup:/Network/Service/wifi0/Proxies", "p").unwrap();
        s.set("State:/Network/Interface/en0/IPv4", "a").unwrap();
        s.set("State:/Network/Global/IPv4", "b").unwrap();
        assert_eq!(
            s.keys("State:/Network/"),
            vec!["State:/Network/Global/IPv4", "State:/Network/Interface/en0/IPv4"]
        );
        assert_eq!(s.keys("Setup:/").len(), 1);
        assert_eq!(s.keys("").len(), 3);
    }
}
