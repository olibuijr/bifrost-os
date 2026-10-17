//! The `records` service: a record store per app, the shell's side of `datastored`. The model is
//! `bifrost_ui::data_store` (a `DataStore` trait, batches that apply in full or not at all, id order on fetch) with
//! `bifrost_ui::model_store::Record` values. Each app sees only its own records: the store is keyed by the caller's
//! app id, which the shell takes from the connection.
//!
//! Shared stores: a request with `suite` names a store that every first-party app can open by that name, the way a
//! shared defaults suite does. A third-party app that names a suite is refused. A store's `revision` moves on every
//! write; `changes {entity, since}` answers `{revision, changed, full, records, removed}`: the records that writes
//! changed since the revision the client holds (from a log of the last `LOG_LIMIT` changed records, kept per store),
//! and the ids of those removed since. A client older than the log gets every record of the entity (`full`). The revision starts from the clock, so one from before a
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
use std::collections::{BTreeMap, BTreeSet};
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

/// How many changed-record entries a store keeps for `changes`. Older ones are dropped, and a client that holds a
/// revision older than the kept log gets the whole entity back instead (`full`).
pub const LOG_LIMIT: usize = 512;

/// One store, its revision, and the log of the records each write changed.
struct Slot {
    store: MemoryDataStore,
    revision: u64,
    /// The changed records since `log_base`, oldest first: one entry per record id a write changed, at its revision.
    log: Vec<Change>,
    /// The revision the log starts after. A client holding a revision from `log_base` on gets exact changes.
    log_base: u64,
}

/// One record a write changed (a put, a removal, or an erase), at the store revision of that write.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    revision: u64,
    entity: String,
    id: u64,
}

impl Slot {
    fn new(store: MemoryDataStore, revision: u64) -> Slot {
        Slot { store, revision, log: Vec::new(), log_base: revision }
    }

    /// Records the records that differ between `before` (the store before a write) and the store now, at `revision`.
    fn log_changes(&mut self, before: &[Record], revision: u64) {
        let after = self.store.all();
        let old: BTreeMap<u64, &Record> = before.iter().map(|r| (r.id, r)).collect();
        let new: BTreeMap<u64, &Record> = after.iter().map(|r| (r.id, r)).collect();
        let ids: BTreeSet<u64> = old.keys().chain(new.keys()).copied().collect();
        for id in ids {
            let (a, b) = (old.get(&id), new.get(&id));
            if a == b {
                continue;
            }
            let entity = b.or(a).map(|r| r.entity.clone()).unwrap_or_default();
            self.log.push(Change { revision, entity, id });
        }
        while self.log.len() > LOG_LIMIT {
            let Some(first) = self.log.first().map(|c| c.revision) else { break };
            let n = self.log.iter().take_while(|c| c.revision == first).count();
            self.log.drain(..n);
            self.log_base = first;
        }
    }
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
            self.stores.insert(key.clone(), Slot::new(store, epoch_now()));
        }
        let slot = self.stores.get_mut(&key).ok_or("records store is missing")?;
        if method == "changes" {
            let entity = text(params, "entity")?;
            let since = params.get("since").and_then(Json::as_u64).unwrap_or(0);
            return Ok(changes_json(slot, since, &entity));
        }
        let writes = matches!(method, "put" | "remove" | "erase");
        let before = if writes { slot.store.all() } else { Vec::new() };
        let out = answer(&mut slot.store, method, params)?;
        if writes {
            slot.revision += 1;
            let revision = slot.revision;
            slot.log_changes(&before, revision);
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

/// The answer to `changes`. When the store did not move past `since`, nothing. Otherwise the records of `entity` that
/// writes changed since `since` (from the log), and the ids of those that are gone in `removed`. A client older than
/// the kept log, or ahead of the store, gets every record of the entity with `full` set.
pub(crate) fn changes_json(slot: &Slot, since: u64, entity: &str) -> Json {
    let changed = slot.revision != since;
    let full = since < slot.log_base || since > slot.revision;
    let mut records = Vec::new();
    let mut removed = Vec::new();
    if changed && full {
        records = slot.store.fetch(entity).iter().map(record_json).collect();
    } else if changed {
        let current = slot.store.all();
        let ids: BTreeSet<u64> = slot.log.iter().filter(|c| c.revision > since && c.entity == entity).map(|c| c.id).collect();
        for id in ids {
            match current.iter().find(|r| r.id == id && r.entity == entity) {
                Some(record) => records.push(record_json(record)),
                None => removed.push(Json::Num(id as f64)),
            }
        }
    }
    Json::Obj(vec![
        ("revision".into(), Json::Num(slot.revision as f64)),
        ("changed".into(), Json::Bool(changed)),
        ("full".into(), Json::Bool(changed && full)),
        ("records".into(), Json::Arr(records)),
        ("removed".into(), Json::Arr(removed)),
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

    /// One write through the service path: runs it, moves the revision and logs what it changed.
    fn write(slot: &mut Slot, method: &str, params: &Json) {
        let before = slot.store.all();
        answer(&mut slot.store, method, params).unwrap();
        slot.revision += 1;
        let revision = slot.revision;
        slot.log_changes(&before, revision);
    }

    fn put_json(id: f64, name: &str) -> Json {
        obj(vec![
            ("id", Json::Num(id)),
            ("entity", Json::Str("Note".into())),
            ("fields", obj(vec![("name", Json::Str(name.into()))])),
        ])
    }

    fn ids(json: &Json, key: &str) -> Vec<u64> {
        json.get(key).and_then(Json::as_array).map(|a| a.iter().filter_map(|v| v.as_u64().or_else(|| v.get("id")?.as_u64())).collect()).unwrap_or_default()
    }

    #[test]
    fn changes_answer_the_records_only_when_the_revision_moved() {
        let mut slot = Slot::new(MemoryDataStore::new(), 10);
        write(&mut slot, "put", &put_json(1.0, "first"));
        write(&mut slot, "put", &put_json(2.0, "second"));
        let all = changes_json(&slot, 10, "Note");
        assert_eq!(all.get("changed"), Some(&Json::Bool(true)));
        assert_eq!(ids(&all, "records"), vec![1, 2]);
        assert_eq!(all.get("full"), Some(&Json::Bool(false)));
        // A client at revision 11 only needs the second record.
        assert_eq!(ids(&changes_json(&slot, 11, "Note"), "records"), vec![2]);
        let same = changes_json(&slot, 12, "Note");
        assert_eq!(same.get("changed"), Some(&Json::Bool(false)));
        assert_eq!(same.get("records").and_then(Json::as_array).map(|r| r.len()), Some(0));
        assert!(epoch_now() > 1_000_000);
    }

    #[test]
    fn a_removed_record_is_named_so_the_client_drops_it() {
        let mut slot = Slot::new(MemoryDataStore::new(), 10);
        write(&mut slot, "put", &put_json(1.0, "first"));
        write(&mut slot, "put", &put_json(2.0, "second"));
        write(&mut slot, "remove", &obj(vec![("id", Json::Num(1.0))]));
        let after = changes_json(&slot, 12, "Note");
        assert!(ids(&after, "records").is_empty());
        assert_eq!(ids(&after, "removed"), vec![1]);
    }

    #[test]
    fn an_erase_names_every_record_it_removed() {
        let mut slot = Slot::new(MemoryDataStore::new(), 10);
        write(&mut slot, "put", &put_json(1.0, "first"));
        write(&mut slot, "put", &put_json(2.0, "second"));
        write(&mut slot, "erase", &obj(vec![]));
        let after = changes_json(&slot, 12, "Note");
        assert_eq!(ids(&after, "removed"), vec![1, 2]);
        assert!(ids(&after, "records").is_empty());
    }

    #[test]
    fn a_client_older_than_the_log_gets_the_whole_entity() {
        let mut slot = Slot::new(MemoryDataStore::new(), 10);
        write(&mut slot, "put", &put_json(1.0, "first"));
        // Trim the log so it starts after revision 11: a client at 5 must be told everything.
        slot.log.clear();
        slot.log_base = 11;
        let old = changes_json(&slot, 5, "Note");
        assert_eq!(old.get("full"), Some(&Json::Bool(true)));
        assert_eq!(ids(&old, "records"), vec![1]);
        assert!(ids(&old, "removed").is_empty());
        // A client at the base still gets exact changes (none are logged after it here).
        assert_eq!(changes_json(&slot, 11, "Note").get("changed"), Some(&Json::Bool(false)));
    }

    #[test]
    fn the_log_is_capped_and_drops_whole_revisions() {
        let mut slot = Slot::new(MemoryDataStore::new(), 0);
        for i in 0..(LOG_LIMIT as u64 + 5) {
            write(&mut slot, "put", &put_json((i + 1) as f64, "x"));
        }
        assert!(slot.log.len() <= LOG_LIMIT);
        assert!(slot.log_base > 0);
        assert!(slot.log.iter().all(|c| c.revision > slot.log_base));
        assert_eq!(changes_json(&slot, 0, "Note").get("full"), Some(&Json::Bool(true)));
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
