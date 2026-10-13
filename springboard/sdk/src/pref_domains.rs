//! Cross-app preference domains: the counterpart of `CFPreferences` and `UserDefaults(suiteName:)`.
//!
//! `defaults.rs` keeps one file per app. A `PrefStore` instead works on a shared host directory with one
//! file per *domain* (an application id, a suite name, or the global domain), so two apps that name the
//! same domain see each other's values after `app_synchronize`, as `CFPreferencesAppSynchronize` does.
//! Writes stay in memory until synchronised; synchronising writes the pending changes, then drops the
//! cached copy so the next read loads the file again and sees other processes' changes. Reads of an app
//! domain fall back to the global domain (`CFPreferencesCopyAppValue`).
//!
//! Files: `<root>/<domain>.txt` (`<root>/AnyUser/...` for `kCFPreferencesAnyUser`, `<root>/ByHost/...` for
//! `kCFPreferencesCurrentHost`), in the `defaults_file` format.

use bifrost_core::defaults_file::{parse, serialise, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `kCFPreferencesAnyApplication`: the global domain.
pub const ANY_APPLICATION: &str = "kCFPreferencesAnyApplication";
/// `kCFPreferencesCurrentApplication`: the store's own application id.
pub const CURRENT_APPLICATION: &str = "kCFPreferencesCurrentApplication";
pub const ANY_USER: &str = "kCFPreferencesAnyUser";
pub const CURRENT_USER: &str = "kCFPreferencesCurrentUser";
pub const ANY_HOST: &str = "kCFPreferencesAnyHost";
pub const CURRENT_HOST: &str = "kCFPreferencesCurrentHost";
/// the global domain.
pub const GLOBAL_DOMAIN: &str = "GlobalDomain";
/// `UserDefaults.didChangeNotification`.
pub const DID_CHANGE_NOTIFICATION: &str = "UserDefaultsDidChange";

type Observer = Box<dyn Fn(&str, &str)>;

/// The preferences of one process over a shared directory.
pub struct PrefStore {
    root: PathBuf,
    app_id: String,
    /// file key (`ByHost/com.x`) -> values as last loaded or written
    cache: BTreeMap<String, BTreeMap<String, Value>>,
    dirty: BTreeSet<String>,
    observers: Vec<Observer>,
}

impl PrefStore {
    /// A store for the application `app_id` over `root`.
    pub fn open(root: &Path, app_id: &str) -> Self {
        PrefStore { root: root.to_path_buf(), app_id: app_id.to_string(), cache: BTreeMap::new(), dirty: BTreeSet::new(), observers: Vec::new() }
    }

    /// The host store under `$HOME/.local/share/bifrost/Preferences`.
    pub fn open_host(app_id: &str) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Self::open(&Path::new(&home).join(".local/share/bifrost/Preferences"), app_id)
    }

    /// `UserDefaults(suiteName:)`: the same store, naming the suite as the application. Apps that open the
    /// same suite share its values.
    pub fn suite(root: &Path, suite_name: &str) -> Self {
        Self::open(root, suite_name)
    }

    pub fn application_id(&self) -> &str {
        &self.app_id
    }

    /// `UserDefaults.didChangeNotification`: `f(domain, key)` runs after every local change; a removed or
    /// replaced key notifies too, a write of an equal value does not.
    pub fn observe(&mut self, f: impl Fn(&str, &str) + 'static) {
        self.observers.push(Box::new(f));
    }

    fn domain_name(&self, app: &str) -> String {
        match app {
            ANY_APPLICATION => GLOBAL_DOMAIN.to_string(),
            CURRENT_APPLICATION => self.app_id.clone(),
            other => other.to_string(),
        }
    }

    fn file_key(&self, app: &str, user: &str, host: &str) -> String {
        let domain = self.domain_name(app);
        let mut prefix = String::new();
        if user == ANY_USER {
            prefix.push_str("AnyUser/");
        }
        if host == CURRENT_HOST {
            prefix.push_str("ByHost/");
        }
        format!("{prefix}{domain}")
    }

    fn path_of(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.txt"))
    }

    fn load(&mut self, key: &str) -> &mut BTreeMap<String, Value> {
        if !self.cache.contains_key(key) {
            let values = std::fs::read_to_string(self.path_of(key)).map(|t| parse(&t)).unwrap_or_default();
            self.cache.insert(key.to_string(), values);
        }
        self.cache.get_mut(key).expect("just inserted")
    }

    /// `CFPreferencesCopyValue`: one domain only, no fallback.
    pub fn copy_value(&mut self, key: &str, app: &str, user: &str, host: &str) -> Option<Value> {
        let fk = self.file_key(app, user, host);
        self.load(&fk).get(key).cloned()
    }

    /// `CFPreferencesCopyAppValue`: the app's domain, then the global domain.
    pub fn copy_app_value(&mut self, key: &str, app: &str) -> Option<Value> {
        match self.copy_value(key, app, CURRENT_USER, ANY_HOST) {
            Some(v) => Some(v),
            None => self.copy_value(key, ANY_APPLICATION, CURRENT_USER, ANY_HOST),
        }
    }

    /// `CFPreferencesSetValue`; `None` removes the key.
    pub fn set_value(&mut self, key: &str, value: Option<Value>, app: &str, user: &str, host: &str) {
        let fk = self.file_key(app, user, host);
        let domain = self.domain_name(app);
        let map = self.load(&fk);
        let changed = match value {
            Some(v) => map.insert(key.to_string(), v.clone()).as_ref() != Some(&v),
            None => map.remove(key).is_some(),
        };
        if changed {
            self.dirty.insert(fk);
            for o in &self.observers {
                o(&domain, key);
            }
        }
    }

    /// `CFPreferencesSetAppValue`.
    pub fn set_app_value(&mut self, key: &str, value: Option<Value>, app: &str) {
        self.set_value(key, value, app, CURRENT_USER, ANY_HOST);
    }

    /// `CFPreferencesCopyKeyList`, sorted.
    pub fn key_list(&mut self, app: &str, user: &str, host: &str) -> Vec<String> {
        let fk = self.file_key(app, user, host);
        self.load(&fk).keys().cloned().collect()
    }

    /// `CFPreferencesAppSynchronize`: writes the pending changes of this app's domains and the global
    /// domain, then forgets their cached copies so the next read reloads the file. False when a write
    /// failed; that domain keeps its pending changes.
    pub fn app_synchronize(&mut self, app: &str) -> bool {
        let domain = self.domain_name(app);
        let keys: Vec<String> = self
            .cache
            .keys()
            .filter(|k| {
                let last = k.rsplit('/').next().unwrap_or("");
                last == domain || last == GLOBAL_DOMAIN
            })
            .cloned()
            .collect();
        let mut ok = true;
        for k in keys {
            if self.dirty.contains(&k) {
                let text = serialise(&self.cache[&k]);
                if bifrost_core::fsutil::write_atomic(&self.path_of(&k), text).is_err() {
                    ok = false;
                    continue;
                }
                self.dirty.remove(&k);
            }
            self.cache.remove(&k);
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn s(v: &str) -> Option<Value> {
        Some(Value::Str(v.into()))
    }

    #[test]
    fn two_apps_share_a_domain_after_synchronize() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        let mut b = PrefStore::open(t.path(), "com.b");
        a.set_app_value("theme", s("dark"), "shared.suite");
        assert_eq!(b.copy_app_value("theme", "shared.suite"), None, "unsynchronised writes stay private");
        b.app_synchronize("shared.suite");
        assert!(a.app_synchronize("shared.suite"));
        assert_eq!(b.copy_app_value("theme", "shared.suite"), s("dark"));
    }

    #[test]
    fn a_reader_sees_changes_after_its_own_synchronize() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        let mut b = PrefStore::open(t.path(), "com.b");
        assert_eq!(b.copy_app_value("k", "d"), None);
        a.set_app_value("k", Some(Value::Int(5)), "d");
        a.app_synchronize("d");
        assert_eq!(b.copy_app_value("k", "d"), None, "b still holds its cached copy");
        b.app_synchronize("d");
        assert_eq!(b.copy_app_value("k", "d"), Some(Value::Int(5)));
    }

    #[test]
    fn the_global_domain_is_the_fallback() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        a.set_app_value("uiLanguage", s("is"), ANY_APPLICATION);
        assert_eq!(a.copy_app_value("uiLanguage", "com.a"), s("is"));
        a.set_app_value("uiLanguage", s("en"), "com.a");
        assert_eq!(a.copy_app_value("uiLanguage", "com.a"), s("en"));
        assert_eq!(a.copy_value("uiLanguage", GLOBAL_DOMAIN, CURRENT_USER, ANY_HOST), s("is"));
        a.set_app_value("uiLanguage", None, "com.a");
        assert_eq!(a.copy_app_value("uiLanguage", "com.a"), s("is"));
    }

    #[test]
    fn current_application_names_the_stores_own_id() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        a.set_app_value("k", s("v"), CURRENT_APPLICATION);
        assert_eq!(a.copy_app_value("k", "com.a"), s("v"));
        assert_eq!(a.key_list("com.a", CURRENT_USER, ANY_HOST), vec!["k"]);
    }

    #[test]
    fn user_and_host_scopes_are_separate_files() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        a.set_value("k", s("host"), "com.a", CURRENT_USER, CURRENT_HOST);
        a.set_value("k", s("any"), "com.a", ANY_USER, ANY_HOST);
        a.set_value("k", s("user"), "com.a", CURRENT_USER, ANY_HOST);
        assert!(a.app_synchronize("com.a"));
        assert!(t.path().join("ByHost/com.a.txt").exists());
        assert!(t.path().join("AnyUser/com.a.txt").exists());
        assert_eq!(a.copy_value("k", "com.a", CURRENT_USER, CURRENT_HOST), s("host"));
        assert_eq!(a.copy_value("k", "com.a", ANY_USER, ANY_HOST), s("any"));
        assert_eq!(a.copy_value("k", "com.a", CURRENT_USER, ANY_HOST), s("user"));
    }

    #[test]
    fn change_notifications_fire_once_per_real_change() {
        let t = tempfile::tempdir().unwrap();
        let mut a = PrefStore::open(t.path(), "com.a");
        let seen: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        a.observe(move |d, k| sink.borrow_mut().push((d.to_string(), k.to_string())));
        a.set_app_value("k", Some(Value::Int(1)), "com.a");
        a.set_app_value("k", Some(Value::Int(1)), "com.a");
        a.set_app_value("k", None, "com.a");
        a.set_app_value("k", None, "com.a");
        assert_eq!(*seen.borrow(), vec![("com.a".to_string(), "k".to_string()); 2]);
        assert_eq!(DID_CHANGE_NOTIFICATION, "UserDefaultsDidChange");
    }

    #[test]
    fn synchronize_on_an_unwritable_root_reports_failure_and_keeps_the_value() {
        let t = tempfile::tempdir().unwrap();
        let blocker = t.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let mut a = PrefStore::open(&blocker, "com.a");
        a.set_app_value("k", s("v"), "com.a");
        assert!(!a.app_synchronize("com.a"));
        assert_eq!(a.copy_app_value("k", "com.a"), s("v"));
    }
}
