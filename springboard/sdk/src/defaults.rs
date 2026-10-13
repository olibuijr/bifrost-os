//! `UserDefaults`: the app's persistent preferences, the counterpart of `NativeUserDefaults.standard`.
//!
//! Values are typed like Foundation's (string, bool, integer, double, data), a missing or
//! mistyped key reads as the zero value (`false`, `0`, `0.0`, empty), and every change is written
//! to `<app data>/defaults.txt` at once with an atomic replace, so a crash keeps the last
//! consistent state. The file is one `kind<TAB>key<TAB>value` line per key; tabs, newlines and
//! backslashes in keys and strings are escaped, and data is hex.

use bifrost_core::defaults_file::{parse, serialise};
use bifrost_core::plist::Plist;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file name inside the app's data directory.
pub const FILE: &str = bifrost_core::defaults_file::FILE;

/// A stored value. The file format and its codec live in `bifrost_core::defaults_file`, shared with the `defaults` tool.
pub use bifrost_core::defaults_file::Value;

/// The defaults of one app. Built by `Ctx::defaults`; `open` is also usable on its own.
#[derive(Debug)]
pub struct UserDefaults {
    path: Option<PathBuf>,
    values: BTreeMap<String, Value>,
    /// the registration domain (`register(defaults:)`): read when a key has no value, never saved
    registered: BTreeMap<String, Value>,
}

impl UserDefaults {
    /// Loads `<dir>/defaults.txt`; a missing or unreadable file is an empty store.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let values = std::fs::read_to_string(&path).map(|t| parse(&t)).unwrap_or_default();
        UserDefaults { path: Some(path), values, registered: BTreeMap::new() }
    }

    /// A store that is never written to disk (tests, and apps with no data directory).
    pub fn memory() -> Self {
        UserDefaults { path: None, values: BTreeMap::new(), registered: BTreeMap::new() }
    }

    /// Fallback values (`register(defaults:)`): a key with no stored value reads its registered one. They live in
    /// memory only, are never written to disk, and a later registration of the same key replaces the earlier one.
    /// Call it at launch, before reading.
    pub fn register(&mut self, defaults: impl IntoIterator<Item = (&'static str, Value)>) {
        self.registered.extend(defaults.into_iter().map(|(k, v)| (k.to_string(), v)));
    }

    /// Whether the key has a stored value (registered fallbacks do not count, like `persistentDomain`).
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key).or_else(|| self.registered.get(key))
    }

    /// Every key, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
    pub fn bool(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Bool(true)))
    }
    pub fn int(&self, key: &str) -> i64 {
        match self.get(key) {
            Some(Value::Int(n)) => *n,
            _ => 0,
        }
    }
    pub fn double(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(Value::Double(d)) => *d,
            _ => 0.0,
        }
    }
    /// Any stored value as a property list: arrays and dictionaries as well as the scalars.
    pub fn plist(&self, key: &str) -> Option<Plist> {
        self.get(key).map(Value::to_plist)
    }
    pub fn set_plist(&mut self, key: &str, v: Plist) {
        self.set(key, Value::from_plist(v));
    }
    pub fn data(&self, key: &str) -> Option<Vec<u8>> {
        match self.get(key)? {
            Value::Data(d) => Some(d.clone()),
            _ => None,
        }
    }

    pub fn set_string(&mut self, key: &str, v: &str) {
        self.set(key, Value::Str(v.to_string()));
    }
    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.set(key, Value::Bool(v));
    }
    pub fn set_int(&mut self, key: &str, v: i64) {
        self.set(key, Value::Int(v));
    }
    pub fn set_double(&mut self, key: &str, v: f64) {
        self.set(key, Value::Double(v));
    }
    pub fn set_data(&mut self, key: &str, v: &[u8]) {
        self.set(key, Value::Data(v.to_vec()));
    }

    /// Removes a key (`removeObject(forKey:)`).
    pub fn remove(&mut self, key: &str) {
        if self.values.remove(key).is_some() {
            self.save();
        }
    }

    fn set(&mut self, key: &str, v: Value) {
        if self.values.get(key) == Some(&v) {
            return;
        }
        self.values.insert(key.to_string(), v);
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let text = serialise(&self.values);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // a failed write keeps the value in memory; the next change tries again
        let _ = bifrost_core::fsutil::write_atomic(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_defaults_are_read_until_a_value_is_set_and_are_never_saved() {
        let t = tempfile::tempdir().unwrap();
        let mut d = UserDefaults::open(t.path());
        d.register([("showSidebar", Value::Bool(true)), ("fontSize", Value::Int(13))]);
        assert!(d.bool("showSidebar") && d.int("fontSize") == 13 && !d.contains("fontSize"));
        d.set_int("fontSize", 15);
        d.remove("showSidebar");
        assert_eq!((d.int("fontSize"), d.bool("showSidebar")), (15, true), "a removed key falls back to its registered value");
        let again = UserDefaults::open(t.path());
        assert_eq!((again.int("fontSize"), again.bool("showSidebar")), (15, false), "registrations are not persisted");
    }

    #[test]
    fn missing_keys_read_as_the_zero_value() {
        let d = UserDefaults::memory();
        assert_eq!(d.string("k"), None);
        assert!(!d.bool("k"));
        assert_eq!(d.int("k"), 0);
        assert_eq!(d.double("k"), 0.0);
        assert_eq!(d.data("k"), None);
        assert!(!d.contains("k"));
        assert!(d.keys().is_empty());
    }

    #[test]
    fn typed_values_round_trip() {
        let mut d = UserDefaults::memory();
        d.set_string("s", "hi");
        d.set_bool("b", true);
        d.set_int("i", -42);
        d.set_double("d", 1.5);
        d.set_data("x", &[0, 1, 255]);
        assert_eq!(d.string("s").as_deref(), Some("hi"));
        assert!(d.bool("b"));
        assert_eq!(d.int("i"), -42);
        assert_eq!(d.double("d"), 1.5);
        assert_eq!(d.data("x"), Some(vec![0, 1, 255]));
        assert_eq!(d.keys(), vec!["b", "d", "i", "s", "x"]);
    }

    #[test]
    fn a_mistyped_read_is_the_zero_value() {
        let mut d = UserDefaults::memory();
        d.set_int("n", 7);
        d.set_string("s", "x");
        assert_eq!(d.string("n"), None);
        assert!(!d.bool("n"));
        assert_eq!(d.double("n"), 0.0);
        assert_eq!(d.data("n"), None);
        assert_eq!(d.int("s"), 0);
        assert!(d.contains("n"));
    }

    #[test]
    fn set_overwrites_and_changes_the_type() {
        let mut d = UserDefaults::memory();
        d.set_int("k", 1);
        d.set_int("k", 2);
        assert_eq!(d.int("k"), 2);
        d.set_string("k", "now text");
        assert_eq!(d.int("k"), 0);
        assert_eq!(d.string("k").as_deref(), Some("now text"));
        assert_eq!(d.keys().len(), 1);
    }

    #[test]
    fn remove_deletes_and_ignores_missing_keys() {
        let mut d = UserDefaults::memory();
        d.set_bool("k", true);
        d.remove("k");
        assert!(!d.contains("k"));
        assert!(!d.bool("k"));
        d.remove("never-there");
        assert!(d.keys().is_empty());
    }

    #[test]
    fn memory_store_never_touches_disk() {
        let mut d = UserDefaults::memory();
        d.set_int("k", 1);
        assert!(d.path.is_none());
    }

    #[test]
    fn open_on_an_empty_or_missing_dir_is_an_empty_store() {
        let t = tempfile::tempdir().unwrap();
        assert!(UserDefaults::open(t.path()).keys().is_empty());
        assert!(UserDefaults::open(&t.path().join("nope")).keys().is_empty());
    }

    #[test]
    fn every_change_persists_and_reloads() {
        let t = tempfile::tempdir().unwrap();
        {
            let mut d = UserDefaults::open(t.path());
            d.set_string("s", "a\tb\nc\\d\re");
            d.set_bool("b", false);
            d.set_int("i", i64::MIN);
            d.set_double("d", -0.1);
            d.set_data("x", b"\x00\xffz");
            d.set_string("we\tird\nkey", "v");
        }
        assert!(t.path().join(FILE).exists());
        let d = UserDefaults::open(t.path());
        assert_eq!(d.string("s").as_deref(), Some("a\tb\nc\\d\re"));
        assert!(d.contains("b") && !d.bool("b"));
        assert_eq!(d.int("i"), i64::MIN);
        assert_eq!(d.double("d"), -0.1);
        assert_eq!(d.data("x"), Some(b"\x00\xffz".to_vec()));
        assert_eq!(d.string("we\tird\nkey").as_deref(), Some("v"));
    }

    #[test]
    fn removal_persists() {
        let t = tempfile::tempdir().unwrap();
        let mut d = UserDefaults::open(t.path());
        d.set_int("a", 1);
        d.set_int("b", 2);
        d.remove("a");
        let d2 = UserDefaults::open(t.path());
        assert_eq!(d2.keys(), vec!["b"]);
    }

    #[test]
    fn double_round_trips_bit_exactly() {
        let t = tempfile::tempdir().unwrap();
        let mut d = UserDefaults::open(t.path());
        d.set_double("inf", f64::INFINITY);
        d.set_double("tiny", f64::MIN_POSITIVE);
        let d2 = UserDefaults::open(t.path());
        assert_eq!(d2.double("inf"), f64::INFINITY);
        assert_eq!(d2.double("tiny"), f64::MIN_POSITIVE);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(FILE), "garbage\nb\tbad\t2\ni\tbad\tx\nx\todd\tabc\nq\tkind\t1\ni\tgood\t5\ns\ttext\thello\n").unwrap();
        let d = UserDefaults::open(t.path());
        assert_eq!(d.keys(), vec!["good", "text"]);
        assert_eq!(d.int("good"), 5);
        assert_eq!(d.string("text").as_deref(), Some("hello"));
    }

    #[test]
    fn empty_values_survive() {
        let t = tempfile::tempdir().unwrap();
        let mut d = UserDefaults::open(t.path());
        d.set_string("s", "");
        d.set_data("x", &[]);
        let d2 = UserDefaults::open(t.path());
        assert_eq!(d2.string("s").as_deref(), Some(""));
        assert_eq!(d2.data("x"), Some(vec![]));
    }
}
