//! The `domains` service: preference values shared between apps, the shell's side of `cfprefsd`. The model is
//! `bifrost_core::pref_domains` (one file per domain under the shared folder, cached until synchronised). Each app
//! reads and writes its own domain, named by its app id. A shared suite (the `UserDefaults(suiteName:)` case) is named
//! by the caller and is open to first-party apps only, so a third-party app cannot read a suite another app uses.
//!
//! Values are text only in this first version. The model also holds numbers, flags and data; a method for those is
//! not built yet, and the reads of such values answer an error rather than a wrong value.
//!
//! Methods (JSON parameters, `key` required):
//! - `get {key, suite?}` answers `{value}`: the text, or `null` when unset. An app's own domain falls back to the
//!   global domain, as `CFPreferencesCopyAppValue` does.
//! - `set {key, value, suite?}` writes the text and synchronises at once, answering `{saved}`.
//! - `remove {key, suite?}` deletes the key and synchronises, answering `{saved}`.
//!
//! The folder is `<apphost root>/domains`.
use super::keychain_groups::first_party;
use super::{Shell, SystemService};
use crate::shell::apphost;
use bifrost_core::defaults_file::Value;
use bifrost_core::json::Json;
use bifrost_core::pref_domains::{PrefStore, ANY_HOST, CURRENT_USER};
use std::path::Path;

/// The service name.
pub const SERVICE: &str = "domains";

/// The folder under the apphost root that holds the domain files.
pub const DIR: &str = "domains";

/// The `domains` service.
pub struct Domains;

impl SystemService for Domains {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let suite = params.str_field("suite");
        if suite.is_some() && !is_first_party(app_id) {
            return Err("shared preference suites are open to first-party apps only".into());
        }
        let root = apphost::root().join(DIR);
        let _ = std::fs::create_dir_all(&root);
        answer(&root, app_id, suite.as_deref(), method, params)
    }
}

/// Whether the installed app `app_id` is a first-party app (the rule the keychain groups use).
fn is_first_party(app_id: &str) -> bool {
    apphost::installed().iter().find(|a| a.manifest.id == app_id).is_some_and(|a| first_party(&a.manifest))
}

/// Runs one method for the app `app_id`, or for the shared suite `suite` when given, in the folder `root`.
pub fn answer(root: &Path, app_id: &str, suite: Option<&str>, method: &str, params: &Json) -> Result<Json, String> {
    let key = params.str_field("key").filter(|k| !k.is_empty()).ok_or("key must be a non-empty string")?;
    let domain = suite.unwrap_or(app_id);
    let mut store = match suite {
        Some(name) => PrefStore::suite(root, name),
        None => PrefStore::open(root, app_id),
    };
    match method {
        "get" => {
            let found = match suite {
                Some(name) => store.copy_value(&key, name, CURRENT_USER, ANY_HOST),
                None => store.copy_app_value(&key, app_id),
            };
            let value = match found {
                None => Json::Null,
                Some(Value::Str(s)) => Json::Str(s),
                Some(_) => return Err(format!("{key} is not text; this service reads text only")),
            };
            Ok(Json::Obj(vec![("value".into(), value)]))
        }
        "set" => {
            let text = params.str_field("value").ok_or("value must be a string")?;
            store.set_value(&key, Some(Value::Str(text)), domain, CURRENT_USER, ANY_HOST);
            Ok(Json::Obj(vec![("saved".into(), Json::Bool(store.app_synchronize(domain)))]))
        }
        "remove" => {
            store.set_value(&key, None, domain, CURRENT_USER, ANY_HOST);
            Ok(Json::Obj(vec![("saved".into(), Json::Bool(store.app_synchronize(domain)))]))
        }
        _ => Err(format!("domains has no method {method}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sb-domains-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn set(root: &Path, app: &str, suite: Option<&str>, key: &str, value: &str) -> Json {
        answer(root, app, suite, "set", &obj(vec![("key", Json::Str(key.into())), ("value", Json::Str(value.into()))])).unwrap()
    }

    fn get(root: &Path, app: &str, suite: Option<&str>, key: &str) -> Json {
        answer(root, app, suite, "get", &obj(vec![("key", Json::Str(key.into()))])).unwrap().get("value").cloned().unwrap()
    }

    #[test]
    fn an_app_domain_is_private_and_survives_a_fresh_store() {
        let root = scratch("own");
        assert_eq!(set(&root, "app.one", None, "theme", "dark").get("saved"), Some(&Json::Bool(true)));
        assert_eq!(get(&root, "app.one", None, "theme"), Json::Str("dark".into()));
        assert_eq!(get(&root, "app.two", None, "theme"), Json::Null);
    }

    #[test]
    fn a_suite_is_shared_by_name_and_remove_clears_it() {
        let root = scratch("suite");
        set(&root, "app.one", Some("group.shared"), "count", "3");
        assert_eq!(get(&root, "app.two", Some("group.shared"), "count"), Json::Str("3".into()));
        answer(&root, "app.two", Some("group.shared"), "remove", &obj(vec![("key", Json::Str("count".into()))])).unwrap();
        assert_eq!(get(&root, "app.one", Some("group.shared"), "count"), Json::Null);
    }

    #[test]
    fn a_missing_key_is_refused() {
        let root = scratch("key");
        assert!(answer(&root, "app.one", None, "get", &Json::Null).is_err());
        assert!(answer(&root, "app.one", None, "frob", &obj(vec![("key", Json::Str("k".into()))])).is_err());
    }
}
