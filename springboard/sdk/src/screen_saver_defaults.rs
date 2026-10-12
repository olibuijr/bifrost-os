//! `ScreenSaverDefaults`: the preferences of one screen saver module. The module's keys live in their own key space
//! of the app's `UserDefaults` store, written as `ScreenSaver.<module>.<key>`, so one module never reads another's
//! values. A dot in a module name becomes `_`, which keeps the prefixes unambiguous.

use crate::defaults::UserDefaults;

/// The leading part of every screen saver key in the store.
pub const KEY_PREFIX: &str = "ScreenSaver";

/// Typed access to one module's keys (`init(forModuleWithName:)`). Reads and writes go to the wrapped store, so
/// changes are saved with its usual atomic write.
#[derive(Debug)]
pub struct ScreenSaverDefaults {
    store: UserDefaults,
    prefix: String,
}

impl ScreenSaverDefaults {
    /// Defaults for the module called `module_name`, kept in `store`.
    pub fn new(store: UserDefaults, module_name: &str) -> Self {
        let module = module_name.replace('.', "_");
        ScreenSaverDefaults { store, prefix: format!("{}.{}.", KEY_PREFIX, module) }
    }

    /// The full key the store uses for `key` in this module.
    pub fn store_key(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }

    /// The wrapped store, for reading values of the whole app.
    pub fn store(&self) -> &UserDefaults {
        &self.store
    }

    /// Gives the store back, for the app to save or share.
    pub fn into_store(self) -> UserDefaults {
        self.store
    }

    pub fn contains(&self, key: &str) -> bool {
        self.store.contains(&self.store_key(key))
    }

    pub fn string(&self, key: &str) -> Option<String> {
        self.store.string(&self.store_key(key))
    }

    pub fn bool(&self, key: &str) -> bool {
        self.store.bool(&self.store_key(key))
    }

    pub fn int(&self, key: &str) -> i64 {
        self.store.int(&self.store_key(key))
    }

    pub fn double(&self, key: &str) -> f64 {
        self.store.double(&self.store_key(key))
    }

    pub fn set_string(&mut self, key: &str, v: &str) {
        let k = self.store_key(key);
        self.store.set_string(&k, v);
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        let k = self.store_key(key);
        self.store.set_bool(&k, v);
    }

    pub fn set_int(&mut self, key: &str, v: i64) {
        let k = self.store_key(key);
        self.store.set_int(&k, v);
    }

    pub fn set_double(&mut self, key: &str, v: f64) {
        let k = self.store_key(key);
        self.store.set_double(&k, v);
    }

    /// Removes one of the module's keys.
    pub fn remove(&mut self, key: &str) {
        let k = self.store_key(key);
        self.store.remove(&k);
    }

    /// The module's own keys, without the module prefix, in sorted order.
    pub fn keys(&self) -> Vec<String> {
        self.store
            .keys()
            .into_iter()
            .filter_map(|k| k.strip_prefix(self.prefix.as_str()).map(str::to_string))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_written_under_the_module_prefix() {
        let mut d = ScreenSaverDefaults::new(UserDefaults::memory(), "Flurry");
        d.set_string("speed", "fast");
        assert_eq!(d.string("speed"), Some("fast".to_string()));
        assert_eq!(d.store().string("ScreenSaver.Flurry.speed"), Some("fast".to_string()));
        assert_eq!(d.store_key("speed"), "ScreenSaver.Flurry.speed");
    }

    #[test]
    fn modules_do_not_see_each_others_keys() {
        let mut a = ScreenSaverDefaults::new(UserDefaults::memory(), "Clock");
        a.set_int("count", 3);
        let store = a.into_store();
        // "Clock2" has the prefix "ScreenSaver.Clock2.", which is not a match for "ScreenSaver.Clock.".
        let b = ScreenSaverDefaults::new(store, "Clock2");
        assert_eq!(b.int("count"), 0);
        assert!(!b.contains("count"));
    }

    #[test]
    fn dots_in_module_names_become_underscores() {
        let mut d = ScreenSaverDefaults::new(UserDefaults::memory(), "a.b");
        d.set_bool("on", true);
        assert_eq!(d.store_key("on"), "ScreenSaver.a_b.on");
        assert!(d.bool("on"));
    }

    #[test]
    fn typed_defaults_read_zero_when_missing() {
        let mut d = ScreenSaverDefaults::new(UserDefaults::memory(), "M");
        assert_eq!(d.int("missing"), 0);
        assert_eq!(d.double("missing"), 0.0);
        assert!(!d.bool("missing"));
        assert_eq!(d.string("missing"), None);
        d.set_int("n", 42);
        d.set_double("ratio", 0.5);
        assert_eq!(d.int("n"), 42);
        assert_eq!(d.double("ratio"), 0.5);
    }

    #[test]
    fn keys_lists_only_this_modules_keys_sorted() {
        let mut d = ScreenSaverDefaults::new(UserDefaults::memory(), "M");
        d.set_string("speed", "fast");
        d.set_int("count", 1);
        let mut other = ScreenSaverDefaults::new(d.into_store(), "N");
        other.set_bool("on", true);
        let m = ScreenSaverDefaults::new(other.into_store(), "M");
        assert_eq!(m.keys(), vec!["count".to_string(), "speed".to_string()]);
    }

    #[test]
    fn remove_clears_a_key() {
        let mut d = ScreenSaverDefaults::new(UserDefaults::memory(), "M");
        d.set_bool("on", true);
        d.remove("on");
        assert!(!d.contains("on"));
        assert!(!d.bool("on"));
    }
}
