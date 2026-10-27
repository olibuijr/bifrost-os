//! `UserDefaults`: the app's persistent preferences, the counterpart of `NativeUserDefaults.standard`.
//!
//! Values are typed like Foundation's (string, bool, integer, double, data), a missing or
//! mistyped key reads as the zero value (`false`, `0`, `0.0`, empty), and every change is written
//! to `<app data>/defaults.txt` at once with an atomic replace, so a crash keeps the last
//! consistent state. The file is one `kind<TAB>key<TAB>value` line per key; tabs, newlines and
//! backslashes in keys and strings are escaped, and data is hex.

use bifrost_core::defaults_file::{parse, serialise};
use bifrost_core::plist::Plist;
use bifrost_core::pref_domains::{PrefStore, ANY_HOST, CURRENT_USER};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file name inside the app's data directory.
pub const FILE: &str = bifrost_core::defaults_file::FILE;

/// A stored value. The file format and its codec live in `bifrost_core::defaults_file`, shared with the `defaults` tool.
pub use bifrost_core::defaults_file::Value;

/// The defaults of one app. Built by `Ctx::defaults`; `open` is also usable on its own.
pub struct UserDefaults {
    path: Option<PathBuf>,
    /// the shared domain store, when built by `with_store`: the app domain, suites and global domain live there
    domains: Option<Domains>,
    values: BTreeMap<String, Value>,
    /// the registration domain (`register(defaults:)`): read when a key has no value, never saved
    registered: BTreeMap<String, Value>,
}

struct Domains {
    store: RefCell<PrefStore>,
    app: String,
    suites: Vec<String>,
}

impl std::fmt::Debug for UserDefaults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserDefaults").field("path", &self.path).field("values", &self.values).finish()
    }
}

impl UserDefaults {
    /// Loads `<dir>/defaults.txt`; a missing or unreadable file is an empty store.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let values = std::fs::read_to_string(&path).map(|t| parse(&t)).unwrap_or_default();
        UserDefaults { path: Some(path), domains: None, values, registered: BTreeMap::new() }
    }

    /// A store that is never written to disk (tests, and apps with no data directory).
    pub fn memory() -> Self {
        UserDefaults { path: None, domains: None, values: BTreeMap::new(), registered: BTreeMap::new() }
    }

    /// Defaults backed by shared preference domains. A read searches the app's own domain, then the added suites
    /// (`add_suite`), then the global domain, then the registered values; a write or removal changes the app
    /// domain only and is saved at once.
    pub fn with_store(store: PrefStore, app_id: &str) -> Self {
        let domains = Domains { store: RefCell::new(store), app: app_id.to_string(), suites: Vec::new() };
        UserDefaults { path: None, domains: Some(domains), values: BTreeMap::new(), registered: BTreeMap::new() }
    }

    /// `addSuite(named:)`: values of the suite are searched after the app's own and before the global domain. A
    /// suite added twice keeps its first position. Has no effect on a store opened with `open` or `memory`.
    pub fn add_suite(&mut self, name: &str) {
        if let Some(d) = &mut self.domains {
            if !d.suites.iter().any(|s| s == name) {
                d.suites.push(name.to_string());
            }
        }
    }

    /// `removeSuite(named:)`.
    pub fn remove_suite(&mut self, name: &str) {
        if let Some(d) = &mut self.domains {
            d.suites.retain(|s| s != name);
        }
    }

    /// `persistentDomain(forName:)`: the stored values of one domain, sorted by key. Registered values are not
    /// part of it. Empty without a store.
    pub fn persistent_domain(&self, name: &str) -> Vec<(String, Value)> {
        let Some(d) = &self.domains else { return Vec::new() };
        let mut st = d.store.borrow_mut();
        let keys = st.key_list(name, CURRENT_USER, ANY_HOST);
        keys.into_iter().filter_map(|k| st.copy_value(&k, name, CURRENT_USER, ANY_HOST).map(|v| (k, v))).collect()
    }

    /// Fallback values (`register(defaults:)`): a key with no stored value reads its registered one. They live in
    /// memory only, are never written to disk, and a later registration of the same key replaces the earlier one.
    /// Call it at launch, before reading.
    pub fn register(&mut self, defaults: impl IntoIterator<Item = (&'static str, Value)>) {
        self.registered.extend(defaults.into_iter().map(|(k, v)| (k.to_string(), v)));
    }

    /// Whether the key has a stored value (registered fallbacks do not count, like `persistentDomain`).
    pub fn contains(&self, key: &str) -> bool {
        match &self.domains {
            Some(d) => d.store.borrow_mut().copy_value(key, &d.app, CURRENT_USER, ANY_HOST).is_some(),
            None => self.values.contains_key(key),
        }
    }

    fn get(&self, key: &str) -> Option<Value> {
        let Some(d) = &self.domains else {
            return self.values.get(key).or_else(|| self.registered.get(key)).cloned();
        };
        let mut st = d.store.borrow_mut();
        let found = std::iter::once(d.app.as_str())
            .chain(d.suites.iter().map(String::as_str))
            .chain(std::iter::once(bifrost_core::pref_domains::GLOBAL_DOMAIN))
            .find_map(|dom| st.copy_value(key, dom, CURRENT_USER, ANY_HOST));
        found.or_else(|| self.registered.get(key).cloned())
    }

    /// Every key of the app's own domain, sorted.
    pub fn keys(&self) -> Vec<String> {
        match &self.domains {
            Some(d) => d.store.borrow_mut().key_list(&d.app, CURRENT_USER, ANY_HOST),
            None => self.values.keys().cloned().collect(),
        }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Bool(true)))
    }
    pub fn int(&self, key: &str) -> i64 {
        match self.get(key) {
            Some(Value::Int(n)) => n,
            _ => 0,
        }
    }
    pub fn double(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(Value::Double(d)) => d,
            _ => 0.0,
        }
    }
    /// Any stored value as a property list: arrays and dictionaries as well as the scalars.
    pub fn plist(&self, key: &str) -> Option<Plist> {
        self.get(key).as_ref().map(Value::to_plist)
    }
    pub fn set_plist(&mut self, key: &str, v: Plist) {
        self.set(key, Value::from_plist(v));
    }
    pub fn data(&self, key: &str) -> Option<Vec<u8>> {
        match self.get(key)? {
            Value::Data(d) => Some(d),
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
        if let Some(d) = &self.domains {
            let mut st = d.store.borrow_mut();
            st.set_app_value(key, None, &d.app);
            st.app_synchronize(&d.app);
            return;
        }
        if self.values.remove(key).is_some() {
            self.save();
        }
    }

    fn set(&mut self, key: &str, v: Value) {
        if let Some(d) = &self.domains {
            let mut st = d.store.borrow_mut();
            st.set_app_value(key, Some(v), &d.app);
            st.app_synchronize(&d.app);
            return;
        }
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

    fn store_defaults(root: &Path, app: &str) -> UserDefaults {
        UserDefaults::with_store(PrefStore::open(root, app), app)
    }

    fn global_set(root: &Path, key: &str, v: Value) {
        let mut g = PrefStore::open(root, "other");
        g.set_value(key, Some(v), bifrost_core::pref_domains::ANY_APPLICATION, CURRENT_USER, ANY_HOST);
        assert!(g.app_synchronize(bifrost_core::pref_domains::ANY_APPLICATION));
    }

    #[test]
    fn global_value_is_visible_and_the_app_value_shadows_it() {
        let t = tempfile::tempdir().unwrap();
        global_set(t.path(), "lang", Value::Str("is".into()));
        let mut d = store_defaults(t.path(), "com.x");
        assert_eq!(d.string("lang").as_deref(), Some("is"));
        assert!(!d.contains("lang"), "the global value is not in the app domain");
        d.set_string("lang", "en");
        assert_eq!(d.string("lang").as_deref(), Some("en"));
        d.remove("lang");
        assert_eq!(d.string("lang").as_deref(), Some("is"), "removal uncovers the global value");
    }

    #[test]
    fn registered_value_is_the_last_fallback() {
        let t = tempfile::tempdir().unwrap();
        let mut d = store_defaults(t.path(), "com.x");
        d.register([("size", Value::Int(13))]);
        assert_eq!(d.int("size"), 13);
        global_set(t.path(), "size", Value::Int(14));
        let mut d = store_defaults(t.path(), "com.x");
        d.register([("size", Value::Int(13))]);
        assert_eq!(d.int("size"), 14, "global beats registered");
    }

    #[test]
    fn suite_sits_between_app_and_global() {
        let t = tempfile::tempdir().unwrap();
        global_set(t.path(), "k", Value::Int(1));
        let mut suite = store_defaults(t.path(), "group.s");
        suite.set_int("k", 2);
        let mut d = store_defaults(t.path(), "com.x");
        assert_eq!(d.int("k"), 1);
        d.add_suite("group.s");
        assert_eq!(d.int("k"), 2);
        d.set_int("k", 3);
        assert_eq!(d.int("k"), 3);
        d.remove("k");
        assert_eq!(d.int("k"), 2);
        d.remove_suite("group.s");
        assert_eq!(d.int("k"), 1);
    }

    #[test]
    fn writes_touch_the_app_domain_only() {
        let t = tempfile::tempdir().unwrap();
        global_set(t.path(), "k", Value::Int(1));
        let mut suite = store_defaults(t.path(), "group.s");
        suite.set_int("k", 2);
        let mut d = store_defaults(t.path(), "com.x");
        d.add_suite("group.s");
        d.set_int("k", 9);
        d.remove("k");
        d.set_int("only", 5);
        let mut st = PrefStore::open(t.path(), "probe");
        assert_eq!(st.copy_value("k", bifrost_core::pref_domains::ANY_APPLICATION, CURRENT_USER, ANY_HOST), Some(Value::Int(1)));
        assert_eq!(st.copy_value("k", "group.s", CURRENT_USER, ANY_HOST), Some(Value::Int(2)));
        assert_eq!(d.keys(), vec!["only"]);
        assert_eq!(d.persistent_domain("com.x"), vec![("only".to_string(), Value::Int(5))]);
        assert_eq!(store_defaults(t.path(), "com.x").int("only"), 5, "saved at once");
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
