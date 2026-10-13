//! The `keychain-groups` service: keychain items shared between the apps that list the same group in
//! `keychain_groups = [...]` in their manifests (`kSecAttrAccessGroup`). The shell speaks for the calling app: its id
//! comes from the connection, and its groups come from its installed manifest. They count only when the app also has
//! the `keychain` permission, so a group never grants an app that could not use the keychain at all.
//!
//! Methods, all with JSON parameters and the secret as hex:
//! - `groups` lists the caller's groups.
//! - `list` takes `{group}` and answers an array of `{service, account, modified}` for every item in the group, sorted
//!   by service then account. No secret is sent. A group the caller is not in answers an empty list.
//! - `add`, `update`, `get`, `delete` take `{group, service, account}`, plus `value` for `add` and `update`. Each
//!   answers `{status, value}`: `status` is the OSStatus (0 for success) and `value` the hex secret of `get`.
//!   A group the caller is not in answers errSecNotAvailable. The group must be named on every call: unlike the
//!   app's own keychain, a call never searches across groups.
//! - A request that is malformed (an unknown method, a missing field, a value that is not hex) is a service error.
//!
//! An `add`, `update` or `delete` that succeeds queues a `changed` event to every app in that group, with
//! `{group, account}`. The event is sent on the next shell tick, so the writer's reply is never held back.
//!
//! The store is `<root>/keychain/groups/<group>.kc` (`keychain::Store::open_group`): one file per group, not per app.
use super::{Event, Shell, SystemService};
use crate::keychain::{hex, unhex, Error, Store};
use crate::shell::apphost::{self, Manifest};
use bifrost_core::json::Json;
use bifrost_proto::KeychainOp;
use std::path::Path;

/// The event a member app receives after another member changed an item.
pub const CHANGED: &str = "changed";

#[derive(Default)]
pub struct KeychainGroups {
    /// `(group, account)` of each successful add, update or delete since the last tick.
    changes: Vec<(String, String)>,
}

impl SystemService for KeychainGroups {
    fn name(&self) -> &'static str {
        "keychain-groups"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let member = apphost::installed().iter().find(|a| a.manifest.id == app_id).map(|a| groups_of(&a.manifest)).unwrap_or_default();
        let done = run(&apphost::root(), &member, method, params)?;
        if let Some(change) = done.changed {
            self.changes.push(change);
        }
        Ok(done.body)
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        let changes = std::mem::take(&mut self.changes);
        if changes.is_empty() {
            return Vec::new();
        }
        let apps = apphost::installed();
        let mut out = Vec::new();
        for (group, account) in changes {
            let body = Json::Obj(vec![("group".into(), Json::Str(group.clone())), ("account".into(), Json::Str(account))]);
            for app in apps.iter().filter(|a| groups_of(&a.manifest).contains(&group)) {
                out.push(Event { app: Some(app.manifest.id.clone()), event: CHANGED.into(), body: body.clone() });
            }
        }
        out
    }
}

/// The groups an app may use: its `keychain_groups`, but only with the `keychain` permission.
pub fn groups_of(m: &Manifest) -> Vec<String> {
    if m.permissions.iter().any(|p| p == "keychain") {
        m.keychain_groups.clone()
    } else {
        Vec::new()
    }
}

/// What one call answered: the reply body, and the `(group, account)` a successful mutation changed.
#[derive(Debug, PartialEq)]
pub struct Done {
    pub body: Json,
    pub changed: Option<(String, String)>,
}

/// The `{status, value}` answer of a keychain outcome.
fn answer(code: i32, value: &[u8]) -> Json {
    Json::Obj(vec![("status".into(), Json::Num(f64::from(code))), ("value".into(), Json::Str(hex(value)))])
}

/// Runs one call for an app whose groups are `member` (empty without the `keychain` permission) against the shell
/// data dir `root`. Pure, so the tests drive it with a temporary directory.
pub fn run(root: &Path, member: &[String], method: &str, params: &Json) -> Result<Done, String> {
    if method == "groups" {
        let body = Json::Arr(member.iter().map(|g| Json::Str(g.clone())).collect());
        return Ok(Done { body, changed: None });
    }
    if method == "list" {
        let group = params.get("group").and_then(|v| v.as_str()).ok_or_else(|| "keychain-groups list needs group".to_string())?;
        if !member.iter().any(|g| g == group) {
            return Ok(Done { body: Json::Arr(Vec::new()), changed: None });
        }
        let rows = Store::open_group(root, group).and_then(|s| s.list()).map_err(|e| format!("keychain-groups list: {e}"))?;
        let body = Json::Arr(
            rows.into_iter()
                .map(|l| {
                    Json::Obj(vec![
                        ("service".into(), Json::Str(l.service)),
                        ("account".into(), Json::Str(l.account)),
                        ("modified".into(), Json::Num(l.modified as f64)),
                    ])
                })
                .collect(),
        );
        return Ok(Done { body, changed: None });
    }
    if !matches!(method, "add" | "get" | "update" | "delete") {
        return Err(format!("keychain-groups has no method {method}"));
    }
    let field = |k: &str| params.get(k).and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| format!("keychain-groups {method} needs {k}"));
    let group = field("group")?;
    let service = field("service")?;
    let account = field("account")?;
    let value = match method {
        "add" | "update" => unhex(&field("value")?).ok_or_else(|| "keychain-groups value must be hex".to_string())?,
        _ => Vec::new(),
    };
    if !member.contains(&group) {
        return Ok(Done { body: answer(Error::NotPermitted.code(), &[]), changed: None });
    }
    let op = match method {
        "add" => KeychainOp::Add { service, account: account.clone(), value },
        "update" => KeychainOp::Update { service, account: account.clone(), value },
        "get" => KeychainOp::Get { service, account: account.clone() },
        _ => KeychainOp::Delete { service, account: account.clone() },
    };
    let mutates = method != "get";
    Ok(match Store::open_group(root, &group).and_then(|s| s.apply(&op)) {
        Ok(v) => Done { body: answer(0, &v), changed: mutates.then_some((group, account)) },
        Err(e) => Done { body: answer(e.code(), &[]), changed: None },
    })
}

/// The group of saved website logins: the Settings Passwords page reads it, and an app that lists it under
/// `keychain_groups` (with the `keychain` permission) can save and fill them. The service name is the site and the
/// account is the user name, so one item is one login.
pub const PASSWORDS_GROUP: &str = "is.bifrost.passwords";

/// One saved login as the Passwords page lists it. No secret: `secret` reads that when a login is revealed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub site: String,
    pub account: String,
    /// Unix time the login was last written (0 when unknown).
    pub modified: u64,
}

/// Every login in the passwords group, sorted by site then account. An unreadable group lists as empty.
pub fn logins(root: &Path) -> Vec<Login> {
    let rows = Store::open_group(root, PASSWORDS_GROUP).and_then(|s| s.list()).unwrap_or_default();
    rows.into_iter().map(|r| Login { site: r.service, account: r.account, modified: r.modified }).collect()
}

/// The saved password of one login, or `None` when it is missing or not text.
pub fn secret(root: &Path, site: &str, account: &str) -> Option<String> {
    let value = Store::open_group(root, PASSWORDS_GROUP).and_then(|s| s.get(site, account)).ok()?;
    String::from_utf8(value).ok()
}

/// Deletes one login from the passwords group (the shell acts for the user, so it needs no app membership).
pub fn remove(root: &Path, site: &str, account: &str) -> Result<(), Error> {
    Store::open_group(root, PASSWORDS_GROUP)?.delete(site, account)
}

/// Saves a login in the passwords group, or replaces the password of the login that exists (Internet Accounts).
pub fn save(root: &Path, site: &str, account: &str, password: &str) -> Result<(), Error> {
    let store = Store::open_group(root, PASSWORDS_GROUP)?;
    match store.add(site, account, password.as_bytes()) {
        Err(Error::DuplicateItem) => store.update(site, account, password.as_bytes()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-kcg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn obj(pairs: &[(&str, &str)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), Json::Str(v.to_string()))).collect())
    }

    fn status(d: &Done) -> f64 {
        match d.body.get("status") {
            Some(Json::Num(n)) => *n,
            other => panic!("no status: {other:?}"),
        }
    }

    fn members(g: &str) -> Vec<String> {
        vec![g.to_string()]
    }

    #[test]
    fn list_names_each_item_without_its_secret() {
        let d = dir("listitems");
        let g = "is.bifrost.passwords";
        let m = members(g);
        let add = |svc: &str, acc: &str, v: &str| obj(&[("group", g), ("service", svc), ("account", acc), ("value", v)]);
        run(&d, &m, "add", &add("b.example", "me", "6869")).unwrap();
        run(&d, &m, "add", &add("a.example", "you", "7365637265")).unwrap();
        let params = obj(&[("group", g)]);
        let done = run(&d, &m, "list", &params).unwrap();
        let Json::Arr(rows) = &done.body else { panic!("list is not an array: {:?}", done.body) };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("service"), Some(&Json::Str("a.example".into())));
        assert_eq!(rows[0].get("account"), Some(&Json::Str("you".into())));
        assert_eq!(rows[1].get("service"), Some(&Json::Str("b.example".into())));
        assert!(rows.iter().all(|r| r.get("value").is_none()), "no secret in a listing");
        assert!(matches!(rows[0].get("modified"), Some(Json::Num(_))));
        assert_eq!(done.changed, None);
        let other = run(&d, &["is.other".to_string()], "list", &params).unwrap();
        assert_eq!(other.body, Json::Arr(Vec::new()), "a group the caller is not in lists nothing");
        assert!(run(&d, &m, "list", &Json::Null).is_err(), "list needs the group");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn groups_lists_the_callers_groups() {
        let d = dir("list");
        let two = vec!["is.a.one".to_string(), "is.a.two".to_string()];
        let done = run(&d, &two, "groups", &Json::Null).unwrap();
        assert_eq!(done.body, Json::Arr(vec![Json::Str("is.a.one".into()), Json::Str("is.a.two".into())]));
        assert_eq!(done.changed, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn two_members_of_one_group_share_an_item() {
        let d = dir("share");
        let g = members("is.example.keys");
        let add = obj(&[("group", "is.example.keys"), ("service", "login"), ("account", "me"), ("value", "6869")]);
        let writer = run(&d, &g, "add", &add).unwrap();
        assert_eq!(status(&writer), 0.0);
        let get = obj(&[("group", "is.example.keys"), ("service", "login"), ("account", "me")]);
        // a second app in the same group reads what the first one stored
        let reader = run(&d, &members("is.example.keys"), "get", &get).unwrap();
        assert_eq!(status(&reader), 0.0);
        assert_eq!(reader.body.get("value"), Some(&Json::Str("6869".into())));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_app_outside_the_group_is_refused_and_writes_nothing() {
        let d = dir("refuse");
        let get = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "a")]);
        let done = run(&d, &members("is.example.other"), "get", &get).unwrap();
        assert_eq!(status(&done), f64::from(Error::NotPermitted.code()));
        let add = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "a"), ("value", "00")]);
        let refused = run(&d, &Vec::<String>::new(), "add", &add).unwrap();
        assert_eq!(status(&refused), f64::from(Error::NotPermitted.code()));
        assert_eq!(refused.changed, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_a_successful_mutation_reports_a_change() {
        let d = dir("changed");
        let g = members("is.example.keys");
        let add = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "me"), ("value", "01")]);
        assert_eq!(run(&d, &g, "add", &add).unwrap().changed, Some(("is.example.keys".into(), "me".into())));
        // a duplicate add fails, and a get never changes anything
        let dup = run(&d, &g, "add", &add).unwrap();
        assert_eq!(status(&dup), f64::from(Error::DuplicateItem.code()));
        assert_eq!(dup.changed, None);
        let get = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "me")]);
        assert_eq!(run(&d, &g, "get", &get).unwrap().changed, None);
        let del = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "me")]);
        assert_eq!(run(&d, &g, "delete", &del).unwrap().changed, Some(("is.example.keys".into(), "me".into())));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_bad_group_name_is_a_param_status_not_a_service_error() {
        let d = dir("badname");
        let g = members("../escape");
        let get = obj(&[("group", "../escape"), ("service", "s"), ("account", "a")]);
        let done = run(&d, &g, "get", &get).unwrap();
        assert_eq!(status(&done), f64::from(Error::Param.code()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn malformed_requests_are_service_errors() {
        let d = dir("malformed");
        let g = members("is.example.keys");
        assert!(run(&d, &g, "fly", &Json::Null).is_err());
        assert!(run(&d, &g, "get", &obj(&[("group", "is.example.keys"), ("service", "s")])).is_err());
        let not_hex = obj(&[("group", "is.example.keys"), ("service", "s"), ("account", "a"), ("value", "zz")]);
        assert!(run(&d, &g, "add", &not_hex).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_group_needs_the_keychain_permission_and_a_manifest_lists_it() {
        let text = "id = \"is.example.a\"\nname = \"A\"\nexec = \"a\"\npermissions = [\"keychain\"]\nkeychain_groups = [\"is.example.g\", \"is.example.h\"]\n";
        let m = apphost::Manifest::parse(text).unwrap();
        assert_eq!(groups_of(&m), vec!["is.example.g".to_string(), "is.example.h".to_string()]);
        let no_perm = apphost::Manifest::parse(&text.replace("permissions = [\"keychain\"]", "permissions = [\"storage\"]")).unwrap();
        assert!(groups_of(&no_perm).is_empty(), "a group without the keychain permission grants nothing");
    }

    #[test]
    fn the_service_is_registered_under_its_name() {
        assert_eq!(KeychainGroups::default().name(), "keychain-groups");
        assert_eq!(CHANGED, "changed");
    }

    #[test]
    fn the_passwords_group_lists_reveals_and_removes_logins() {
        let d = dir("passwords");
        let g = members(PASSWORDS_GROUP);
        let add = |site: &str, account: &str, value: &str| {
            let p = obj(&[("group", PASSWORDS_GROUP), ("service", site), ("account", account), ("value", value)]);
            assert_eq!(status(&run(&d, &g, "add", &p).unwrap()), 0.0);
        };
        add("b.example", "bo", "7365637265742d32");
        add("a.example", "alice", "68756e74657232");
        let rows = logins(&d);
        let sites: Vec<(&str, &str)> = rows.iter().map(|l| (l.site.as_str(), l.account.as_str())).collect();
        assert_eq!(sites, vec![("a.example", "alice"), ("b.example", "bo")]);
        assert_eq!(secret(&d, "a.example", "alice").as_deref(), Some("hunter2"));
        assert_eq!(secret(&d, "a.example", "nobody"), None);
        remove(&d, "a.example", "alice").unwrap();
        assert_eq!(logins(&d).len(), 1);
        assert_eq!(remove(&d, "a.example", "alice"), Err(Error::ItemNotFound));
        let _ = std::fs::remove_dir_all(&d);
    }
}
