//! The `records` service: a record store per app, the shell's side of `datastored`. The model is
//! `bifrost_ui::data_store` (a `DataStore` trait, batches that apply in full or not at all, id order on fetch) with
//! `bifrost_ui::model_store::Record` values. Each app sees only its own records: the store is keyed by the caller's
//! app id, which the shell takes from the connection.
//!
//! Shared stores: a request with `suite` names a store that every first-party app can open by that name, the way a
//! shared defaults suite does. A third-party app that names a suite is refused. A store's `revision` moves on every
//! write; `changes {entity, since}` answers `{revision, changed, records}`, with the records of the entity only when
//! the store changed since the revision the client holds. The revision starts from the clock, so one from before a
//! restart is never mistaken for the current one.
//!
//! Storage is in memory, and each app's records are also written to `<apphost root>/records/<app id>.json` after every
//! change (a temporary file, then a rename), so they survive a restart. The file is read once, when the app first
//! calls the service. An app id that is not a plain name (letters, digits, `.`, `-`, `_`, not starting with `.`) gets
//! no file and stays in memory. A damaged record in the file is left out. A write whose file cannot be saved answers
//! an error, though the change stays in memory until the shell restarts.
//!
//! Methods (JSON parameters; ids are whole numbers from 1):
//! - `fetch {entity}` answers `{records: [{id, fields}]}` in id order, `fields` being an object of text values.
//! - `count {entity}` answers `{count}`.
//! - `put {id, entity, fields}` writes one record, replacing the record with that id.
//! - `remove {id}` deletes one record. An id the store does not hold is a service error.
//! - `erase {}` deletes every record of the app.
//!
//! Every write is one batch, so a refused write changes nothing.
use super::keychain_groups::first_party;
use super::location::now_unix;
use super::{Shell, SystemService};
use crate::shell::apphost;
use bifrost_core::json::{parse, Json};
use bifrost_ui::data_store::{Batch, DataStore, DataStoreError, MemoryDataStore};
use bifrost_ui::model_store::Record;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The service name.
pub const SERVICE: &str = "records";

/// The folder under the apphost root that holds one record file per app.
pub const DIR: &str = "records";

/// The `records` service: one store per app id.
#[derive(Default)]
pub struct RecordStore {
    stores: BTreeMap<String, Slot>,
}

/// One store and its revision.
struct Slot {
    store: MemoryDataStore,
    revision: u64,
}

impl SystemService for RecordStore {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let installed = crate::shell::apphost::installed();
        let app_first_party = installed.iter().find(|a| a.manifest.id == app_id).is_some_and(|a| first_party(&a.manifest));
        let key = shared_key(app_id, params.str_field("suite").as_deref(), app_first_party)?;
        let file = store_file(&apphost::root().join(DIR), &key);
        if !self.stores.contains_key(&key) {
            let store = file.as_deref().map(load).unwrap_or_default();
            self.stores.insert(key.clone(), Slot { store, revision: epoch_now() });
        }
        let slot = self.stores.get_mut(&key).ok_or("records store is missing")?;
        if method == "changes" {
            let entity = text(params, "entity")?;
            let since = params.get("since").and_then(Json::as_u64).unwrap_or(0);
            return Ok(changes_json(&slot.store, slot.revision, since, &entity));
        }
        let out = answer(&mut slot.store, method, params)?;
        if matches!(method, "put" | "remove" | "erase") {
            slot.revision += 1;
            if let Some(path) = &file {
                persist(path, &slot.store).map_err(|e| format!("records could not be saved: {e}"))?;
            }
        }
        Ok(out)
    }
}

/// The key of the store a request uses: the app's own store, or the shared store `suite` (first-party apps only).
pub fn shared_key(app_id: &str, suite: Option<&str>, first_party_app: bool) -> Result<String, String> {
    match suite {
        None => Ok(app_id.to_string()),
        Some(_) if !first_party_app => Err("shared record stores are open to first-party apps only".into()),
        Some(name) => Ok(format!("suite.{name}")),
    }
}

/// The starting revision of a store: the clock in seconds, scaled, so it is newer than any revision of an earlier run.
pub fn epoch_now() -> u64 {
    (now_unix().max(0.0) as u64) * 1_000_000
}

/// The answer to `changes`: the records of `entity` when the store moved past `since`, else none.
pub fn changes_json(store: &MemoryDataStore, revision: u64, since: u64, entity: &str) -> Json {
    let changed = revision != since;
    let records = if changed { store.fetch(entity).iter().map(record_json).collect() } else { Vec::new() };
    Json::Obj(vec![
        ("revision".into(), Json::Num(revision as f64)),
        ("changed".into(), Json::Bool(changed)),
        ("records".into(), Json::Arr(records)),
    ])
}

/// The file that holds the records of `app_id` under `root`, or `None` when the id is not a plain name.
pub fn store_file(root: &Path, app_id: &str) -> Option<PathBuf> {
    let plain = !app_id.is_empty()
        && !app_id.starts_with('.')
        && app_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    plain.then(|| root.join(format!("{app_id}.json")))
}

/// Reads the records in `path`. A missing or unreadable file, and a damaged record, leave the store empty or short.
pub fn load(path: &Path) -> MemoryDataStore {
    let mut store = MemoryDataStore::new();
    let Ok(text) = std::fs::read_to_string(path) else { return store };
    let Ok(Json::Arr(items)) = parse(&text) else { return store };
    let mut batch = Batch::new();
    for item in &items {
        if let Some(record) = record_from_file(item) {
            batch.put(record);
        }
    }
    let _ = store.save(&batch);
    store
}

/// Writes every record of `store` to `path`: a temporary file, then a rename, so a crash leaves the old file.
pub fn persist(path: &Path, store: &MemoryDataStore) -> Result<(), String> {
    let items = store.all().iter().map(record_json).collect();
    let text = Json::Arr(items).encode();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn record_from_file(item: &Json) -> Option<Record> {
    let id = item.get("id")?.as_u64()?;
    let entity = item.str_field("entity")?;
    let fields = match item.get("fields")? {
        Json::Obj(pairs) => pairs
            .iter()
            .map(|(k, v)| match v {
                Json::Str(s) => Some((k.clone(), s.clone())),
                _ => None,
            })
            .collect::<Option<BTreeMap<String, String>>>()?,
        _ => return None,
    };
    Some(Record { id, entity, fields })
}

/// Runs one method against one app's store.
pub fn answer(store: &mut MemoryDataStore, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "fetch" => {
            let entity = text(params, "entity")?;
            let records = store.fetch(&entity).iter().map(record_json).collect();
            Ok(Json::Obj(vec![("records".into(), Json::Arr(records))]))
        }
        "count" => {
            let entity = text(params, "entity")?;
            Ok(Json::Obj(vec![("count".into(), Json::Num(store.fetch_count(&entity) as f64))]))
        }
        "put" => {
            let record = record_from(params)?;
            let mut batch = Batch::new();
            batch.put(record);
            store.save(&batch).map_err(describe)?;
            Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
        }
        "remove" => {
            let id = whole(params, "id")?;
            let mut batch = Batch::new();
            batch.remove(id);
            store.save(&batch).map_err(describe)?;
            Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
        }
        "erase" => {
            store.erase();
            Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
        }
        _ => Err(format!("records has no method {method}")),
    }
}

fn describe(e: DataStoreError) -> String {
    match e {
        DataStoreError::NotFound(id) => format!("no record {id}"),
        DataStoreError::InvalidId(id) => format!("record id {id} is not valid; ids start at 1"),
    }
}

fn text(params: &Json, key: &str) -> Result<String, String> {
    params.str_field(key).ok_or_else(|| format!("{key} must be a string"))
}

fn whole(params: &Json, key: &str) -> Result<u64, String> {
    params.get(key).and_then(Json::as_u64).ok_or_else(|| format!("{key} must be a whole number"))
}

fn record_from(params: &Json) -> Result<Record, String> {
    let fields = match params.get("fields") {
        Some(Json::Obj(pairs)) => pairs
            .iter()
            .map(|(k, v)| match v {
                Json::Str(s) => Ok((k.clone(), s.clone())),
                _ => Err(format!("field {k} must be text")),
            })
            .collect::<Result<BTreeMap<String, String>, String>>()?,
        _ => return Err("fields must be an object".to_string()),
    };
    Ok(Record { id: whole(params, "id")?, entity: text(params, "entity")?, fields })
}

fn record_json(record: &Record) -> Json {
    let fields = record.fields.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect();
    Json::Obj(vec![("id".into(), Json::Num(record.id as f64)), ("fields".into(), Json::Obj(fields))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn put(store: &mut MemoryDataStore, id: f64, name: &str) -> Result<Json, String> {
        let fields = obj(vec![("name", Json::Str(name.into()))]);
        answer(store, "put", &obj(vec![("id", Json::Num(id)), ("entity", Json::Str("Note".into())), ("fields", fields)]))
    }

    #[test]
    fn a_shared_store_is_for_first_party_apps_only() {
        assert_eq!(shared_key("app.notes", None, false).unwrap(), "app.notes");
        assert!(shared_key("app.other", Some("group.shared"), false).is_err());
        assert_eq!(shared_key("app.notes", Some("group.shared"), true).unwrap(), "suite.group.shared");
    }

    #[test]
    fn changes_answer_the_records_only_when_the_revision_moved() {
        let mut store = MemoryDataStore::new();
        put(&mut store, 1.0, "first").unwrap();
        let moved = changes_json(&store, 7, 3, "Note");
        assert_eq!(moved.get("changed"), Some(&Json::Bool(true)));
        assert_eq!(moved.get("records").and_then(Json::as_array).map(|r| r.len()), Some(1));
        let same = changes_json(&store, 7, 7, "Note");
        assert_eq!(same.get("changed"), Some(&Json::Bool(false)));
        assert_eq!(same.get("records").and_then(Json::as_array).map(|r| r.len()), Some(0));
        assert!(epoch_now() > 1_000_000);
    }

    #[test]
    fn records_survive_a_reload_from_their_file() {
        let dir = std::env::temp_dir().join(format!("sb-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = store_file(&dir, "com.example.notes").unwrap();
        let mut store = MemoryDataStore::new();
        put(&mut store, 1.0, "first").unwrap();
        put(&mut store, 2.0, "second").unwrap();
        persist(&path, &store).unwrap();
        let again = load(&path);
        assert_eq!(again.fetch_count("Note"), 2);
        assert_eq!(again.fetch("Note")[1].fields.get("name").map(String::as_str), Some("second"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_loads_what_it_can_and_names_must_be_plain() {
        let dir = std::env::temp_dir().join(format!("sb-records-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.json");
        std::fs::write(&path, r#"[{"id":3,"entity":"Note","fields":{"name":"ok"}},{"id":"x"},42]"#).unwrap();
        assert_eq!(load(&path).fetch_count("Note"), 1);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path).fetch_count("Note"), 0);
        assert!(store_file(&dir, "../escape").is_none());
        assert!(store_file(&dir, "").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_fetch_remove_and_erase() {
        let mut store = MemoryDataStore::new();
        put(&mut store, 2.0, "second").unwrap();
        put(&mut store, 1.0, "first").unwrap();
        let all = answer(&mut store, "fetch", &obj(vec![("entity", Json::Str("Note".into()))])).unwrap();
        let rows = all.get("records").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("id").and_then(Json::number), Some(1.0));
        let count = answer(&mut store, "count", &obj(vec![("entity", Json::Str("Note".into()))])).unwrap();
        assert_eq!(count.get("count").and_then(Json::number), Some(2.0));
        answer(&mut store, "remove", &obj(vec![("id", Json::Num(1.0))])).unwrap();
        assert!(answer(&mut store, "remove", &obj(vec![("id", Json::Num(1.0))])).is_err());
        answer(&mut store, "erase", &Json::Null).unwrap();
        assert_eq!(store.fetch_count("Note"), 0);
    }

    #[test]
    fn invalid_ids_and_non_text_fields_change_nothing() {
        let mut store = MemoryDataStore::new();
        assert!(put(&mut store, 0.0, "zero").is_err());
        let bad = obj(vec![("id", Json::Num(3.0)), ("entity", Json::Str("Note".into())), ("fields", obj(vec![("n", Json::Num(1.0))]))]);
        assert!(answer(&mut store, "put", &bad).is_err());
        assert_eq!(store.fetch_count("Note"), 0);
    }
}
