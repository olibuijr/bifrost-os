//! The `domains` service: preference values shared between apps, the shell's side of `cfprefsd`. The model is
//! `bifrost_core::pref_domains` (one file per domain under the shared folder, cached until synchronised). Each app
//! reads and writes its own domain, named by its app id. A shared suite (the `UserDefaults(suiteName:)` case) is named
//! by the caller and is open to first-party apps only, so a third-party app cannot read a suite another app uses.
//!
//! Values are text, flags (JSON booleans) and numbers (JSON numbers; a whole number is stored as an integer, any
//! other as a double). Data is hex text: `data` on a set, and `value` as hex with `kind` "data" on a get. Property lists (arrays and
//! dictionaries) are sent as `plist` JSON and answered as the same JSON with `kind` "plist", through
//! `bifrost_core::plist`.
//!
//! Methods (JSON parameters, `key` required):
//! - `get {key, suite?}` answers `{value, kind}`: the text, flag, number, hex data or property list, or `null` when
//!   unset, and `kind` (`text`, `flag`, `number`, `data`, `plist`). An app's own domain falls back to the global domain, as
//!   `CFPreferencesCopyAppValue` does.
//! - `set {key, value, suite?}`, `set {key, data, suite?}` (`data` is hex) or `set {key, plist, suite?}` writes the
//!   value and synchronises at once, answering `{saved}`.
//! - `remove {key, suite?}` deletes the key and synchronises, answering `{saved}`.
//!
//! The folder is `<apphost root>/domains`.
use super::keychain_groups::first_party;
use super::{Shell, SystemService};
use crate::shell::apphost;
use bifrost_core::defaults_file::Value;
use bifrost_core::json::{parse, Json};
use bifrost_core::plist::{self, hex_decode, hex_encode};
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
            let (value, kind) = match found {
                None => (Json::Null, Json::Null),
                Some(Value::Str(s)) => (Json::Str(s), Json::Str("text".into())),
                Some(Value::Bool(b)) => (Json::Bool(b), Json::Str("flag".into())),
                Some(Value::Int(i)) => (Json::Num(i as f64), Json::Str("number".into())),
                Some(Value::Double(d)) => (Json::Num(d), Json::Str("number".into())),
                Some(Value::Data(bytes)) => (Json::Str(hex_encode(&bytes)), Json::Str("data".into())),
                Some(Value::Plist(p)) => {
                    let text = plist::to_json(&p, false)?;
                    (parse(&text)?, Json::Str("plist".into()))
                }
            };
            Ok(Json::Obj(vec![("value".into(), value), ("kind".into(), kind)]))
        }
        "set" => {
            let stored = match params.get("value") {
                _ if params.get("plist").is_some() => Value::from_plist(plist::from_json(&params.get("plist").cloned().unwrap_or(Json::Null))?),
                _ if params.get("data").is_some() => match params.str_field("data").and_then(|h| hex_decode(&h)) {
                    Some(bytes) => Value::Data(bytes),
                    None => return Err("data must be hex text".into()),
                },
                Some(Json::Str(s)) => Value::Str(s.clone()),
                Some(Json::Bool(b)) => Value::Bool(*b),
                Some(Json::Num(n)) if !n.is_finite() => return Err("value must be a finite number".into()),
                Some(Json::Num(n)) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::Int(*n as i64),
                Some(Json::Num(n)) => Value::Double(*n),
                _ => return Err("value must be a string, a flag or a number".into()),
            };
            store.set_value(&key, Some(stored), domain, CURRENT_USER, ANY_HOST);
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
    fn flags_and_numbers_round_trip_with_their_type() {
        let root = scratch("typed");
        let set_json = |key: &str, v: Json| answer(&root, "app.one", None, "set", &obj(vec![("key", Json::Str(key.into())), ("value", v)])).unwrap();
        set_json("enabled", Json::Bool(true));
        set_json("count", Json::Num(7.0));
        set_json("ratio", Json::Num(0.25));
        assert_eq!(get(&root, "app.one", None, "enabled"), Json::Bool(true));
        assert_eq!(get(&root, "app.one", None, "count"), Json::Num(7.0));
        assert_eq!(get(&root, "app.one", None, "ratio"), Json::Num(0.25));
        assert!(answer(&root, "app.one", None, "set", &obj(vec![("key", Json::Str("n".into())), ("value", Json::Null)])).is_err());
    }

    #[test]
    fn data_values_round_trip_as_hex() {
        let root = scratch("data");
        let key = || obj(vec![("key", Json::Str("blob".into()))]);
        let with_data = |hex: &str| obj(vec![("key", Json::Str("blob".into())), ("data", Json::Str(hex.into()))]);
        assert_eq!(answer(&root, "app.one", None, "set", &with_data("00ff10")).unwrap().get("saved"), Some(&Json::Bool(true)));
        let got = answer(&root, "app.one", None, "get", &key()).unwrap();
        assert_eq!(got.get("value"), Some(&Json::Str("00ff10".into())));
        assert_eq!(got.get("kind"), Some(&Json::Str("data".into())));
        assert!(answer(&root, "app.one", None, "set", &with_data("zz")).is_err());
    }

    #[test]
    fn property_lists_round_trip_as_their_json_shape() {
        let root = scratch("plist");
        let dict = Json::Obj(vec![("name".into(), Json::Str("x".into()))]);
        let arr = Json::Arr(vec![Json::Str("a".into()), Json::Str("b".into())]);
        for (key, v) in [("dict", dict.clone()), ("list", arr.clone())] {
            let set = obj(vec![("key", Json::Str(key.into())), ("plist", v)]);
            assert_eq!(answer(&root, "app.one", None, "set", &set).unwrap().get("saved"), Some(&Json::Bool(true)));
        }
        let got = answer(&root, "app.one", None, "get", &obj(vec![("key", Json::Str("dict".into()))])).unwrap();
        assert_eq!(got.get("value"), Some(&dict));
        assert_eq!(got.get("kind"), Some(&Json::Str("plist".into())));
        let got = answer(&root, "app.one", None, "get", &obj(vec![("key", Json::Str("list".into()))])).unwrap();
        assert_eq!(got.get("value"), Some(&arr));
    }

    #[test]
    fn a_missing_key_is_refused() {
        let root = scratch("key");
        assert!(answer(&root, "app.one", None, "get", &Json::Null).is_err());
        assert!(answer(&root, "app.one", None, "frob", &obj(vec![("key", Json::Str("k".into()))])).is_err());
    }
}
