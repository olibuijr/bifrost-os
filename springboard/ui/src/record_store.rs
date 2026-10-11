//! Record database: zones, records, references, queries and change tokens for an app's synced data.
//!
//! Mirrors the reference record model. A `Database` holds named zones; a zone holds `Record`s keyed by record name. A
//! record is a dictionary of typed fields plus a change tag the store sets on every save. A `Reference` links two
//! records in the same zone, and `delete_self` deletes the referring record with its target. A `Predicate` selects
//! records of one type. Every save and delete in a zone takes the next change token, so `changes_since` returns only
//! what a client has not seen yet. Pure data: no network and no disk. A remote server would use the same types.
//!
//! Change tags and change tokens are sequence numbers this module assigns, not opaque server tokens. A tag equals the
//! zone token of the save that wrote it.
//!
//! Gaps: no authorisation per scope (`Scope` is a label only), no sort descriptors, no result cursors, no field
//! indexes, no schema registry (record types and fields are open, as they are during development), and no batch
//! partial-failure report (a batch is applied one record at a time by `record_sync`).

use std::collections::BTreeMap;

use crate::record_asset::Asset;
use crate::record_sync::{ErrorCode, RecordError};

/// The zone every database has (`defaultRecordZone`).
pub const DEFAULT_ZONE: &str = "_defaultZone";

/// How many changes a zone keeps for incremental fetches before the oldest are dropped.
pub const DEFAULT_LOG_LIMIT: usize = 1000;

/// Which partition of a database a record belongs to (`CKDatabase.Scope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Public,
    Private,
    Shared,
}

/// The identity of a record: its zone and its name (`CKRecord.ID`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordID {
    pub zone: String,
    pub name: String,
}

impl RecordID {
    pub fn new(zone: impl Into<String>, name: impl Into<String>) -> Self {
        RecordID { zone: zone.into(), name: name.into() }
    }
}

/// A link to another record in the same zone (`CKRecord.Reference`). `delete_self` is the `deleteSelf` action: when
/// the target is deleted, the record holding this reference is deleted too.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub target: RecordID,
    pub delete_self: bool,
}

/// A field value. `Asset` holds only the handle; the bytes live in an `AssetStore`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    /// Seconds since the Unix epoch.
    Date(u64),
    Ref(Reference),
    Asset(Asset),
}

/// A record: a typed dictionary of fields (`CKRecord`).
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: RecordID,
    pub record_type: String,
    pub fields: BTreeMap<String, Value>,
    /// Set by the store on every save; 0 for a record that was never saved.
    pub change_tag: u64,
    changed: Vec<String>,
}

impl Record {
    pub fn new(id: RecordID, record_type: impl Into<String>) -> Self {
        Record { id, record_type: record_type.into(), fields: BTreeMap::new(), change_tag: 0, changed: Vec::new() }
    }

    /// Sets a field and marks its key as changed (`setObject(_:forKey:)`).
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if !self.changed.contains(&key) {
            self.changed.push(key.clone());
        }
        self.fields.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// Keys set since the record was last saved (`changedKeys()`).
    pub fn changed_keys(&self) -> &[String] {
        &self.changed
    }
}

/// A query condition (the predicate of a query).
#[derive(Clone, Debug, PartialEq)]
pub enum Predicate {
    All,
    Equals(String, Value),
    Not(Box<Predicate>),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
}

impl Predicate {
    pub fn matches(&self, record: &Record) -> bool {
        match self {
            Predicate::All => true,
            Predicate::Equals(key, value) => record.fields.get(key) == Some(value),
            Predicate::Not(p) => !p.matches(record),
            Predicate::And(ps) => ps.iter().all(|p| p.matches(record)),
            Predicate::Or(ps) => ps.iter().any(|p| p.matches(record)),
        }
    }
}

/// What changed in a zone after a token: the records saved since, and the records deleted since. `full` is true when
/// the set is a complete snapshot rather than a delta.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet {
    pub token: u64,
    pub full: bool,
    pub saved: Vec<Record>,
    pub deleted: Vec<RecordID>,
}

#[derive(Clone, Debug, PartialEq)]
enum Change {
    Saved(String),
    Deleted(String),
}

#[derive(Clone, Debug, Default)]
struct Zone {
    /// Token of the latest change; 0 while nothing was ever saved.
    token: u64,
    /// Highest token dropped from the log. A client that last saw a token below this must take a snapshot.
    floor: u64,
    records: BTreeMap<String, Record>,
    log: Vec<(u64, Change)>,
}

/// A record database with named zones (`CKDatabase`).
#[derive(Debug)]
pub struct Database {
    scope: Scope,
    zones: BTreeMap<String, Zone>,
    log_limit: usize,
}

impl Database {
    pub fn new(scope: Scope) -> Self {
        Self::with_log_limit(scope, DEFAULT_LOG_LIMIT)
    }

    /// A database that keeps `limit` changes per zone (at least 1). Older changes are dropped, and a client that
    /// asks for them gets `ChangeTokenExpired`.
    pub fn with_log_limit(scope: Scope, limit: usize) -> Self {
        let mut zones = BTreeMap::new();
        zones.insert(DEFAULT_ZONE.to_string(), Zone::default());
        Database { scope, zones, log_limit: limit.max(1) }
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Creates a zone. Creating an existing zone does nothing.
    pub fn add_zone(&mut self, name: &str) -> Result<(), RecordError> {
        if name.is_empty() {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "a zone name must not be empty"));
        }
        self.zones.entry(name.to_string()).or_default();
        Ok(())
    }

    /// The token of the latest change in `zone`.
    pub fn zone_token(&self, zone: &str) -> Result<u64, RecordError> {
        Ok(self.zone(zone)?.token)
    }

    pub fn fetch(&self, id: &RecordID) -> Result<&Record, RecordError> {
        self.zone(&id.zone)?
            .records
            .get(&id.name)
            .ok_or_else(|| RecordError::new(ErrorCode::UnknownItem, format!("no record named {:?}", id.name)))
    }

    /// Saves a record without a precondition. Returns its new change tag.
    pub fn save(&mut self, record: Record) -> Result<u64, RecordError> {
        self.save_expecting(record, None)
    }

    /// Saves a record if its stored change tag matches `expected_tag`:
    /// `None` saves unconditionally, `Some(0)` creates a record that must not exist yet, and `Some(t)` replaces a
    /// record whose tag is still `t`. Any other state fails with `ServerRecordChanged` (or `UnknownItem` when a
    /// non-zero tag names a record that does not exist).
    pub fn save_expecting(&mut self, mut record: Record, expected_tag: Option<u64>) -> Result<u64, RecordError> {
        self.check_save(&record, expected_tag)?;
        let limit = self.log_limit;
        let zone = self.zone_mut(&record.id.zone)?;
        zone.token += 1;
        let tag = zone.token;
        record.change_tag = tag;
        record.changed.clear();
        let name = record.id.name.clone();
        zone.records.insert(name.clone(), record);
        zone.log.push((tag, Change::Saved(name)));
        trim(zone, limit);
        Ok(tag)
    }

    /// Deletes a record, and every record that refers to it with `delete_self`. Returns the zone token after the
    /// deletion.
    pub fn delete(&mut self, id: &RecordID) -> Result<u64, RecordError> {
        let limit = self.log_limit;
        let zone = self.zone_mut(&id.zone)?;
        if !zone.records.contains_key(&id.name) {
            return Err(RecordError::new(ErrorCode::UnknownItem, format!("no record named {:?}", id.name)));
        }
        let mut doomed: Vec<String> = Vec::new();
        let mut pending = vec![id.name.clone()];
        while let Some(name) = pending.pop() {
            if doomed.contains(&name) {
                continue;
            }
            let children: Vec<String> = zone
                .records
                .values()
                .filter(|r| {
                    r.fields.values().any(|v| matches!(v, Value::Ref(rf) if rf.delete_self && rf.target.name == name))
                })
                .map(|r| r.id.name.clone())
                .collect();
            doomed.push(name);
            pending.extend(children);
        }
        for name in doomed {
            if zone.records.remove(&name).is_some() {
                zone.token += 1;
                zone.log.push((zone.token, Change::Deleted(name)));
            }
        }
        trim(zone, limit);
        Ok(zone.token)
    }

    /// Records of `record_type` in `zone` that match `predicate`, in record-name order.
    pub fn query(&self, zone: &str, record_type: &str, predicate: &Predicate) -> Result<Vec<&Record>, RecordError> {
        Ok(self
            .zone(zone)?
            .records
            .values()
            .filter(|r| r.record_type == record_type && predicate.matches(r))
            .collect())
    }

    /// Every record in `zone`, as a full snapshot.
    pub fn snapshot(&self, zone: &str) -> Result<ChangeSet, RecordError> {
        let z = self.zone(zone)?;
        Ok(ChangeSet { token: z.token, full: true, saved: z.records.values().cloned().collect(), deleted: Vec::new() })
    }

    /// What changed in `zone` after `since`. A record saved and then deleted (or deleted and saved again) appears once,
    /// in its latest state. Fails with `ChangeTokenExpired` when the log no longer covers `since`, and with
    /// `InvalidArguments` when `since` is ahead of the zone.
    pub fn changes_since(&self, zone: &str, since: u64) -> Result<ChangeSet, RecordError> {
        let z = self.zone(zone)?;
        if since > z.token {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "the change token is ahead of the zone"));
        }
        if since < z.floor {
            return Err(RecordError::new(ErrorCode::ChangeTokenExpired, "the change token is too old; take a snapshot"));
        }
        let mut latest: BTreeMap<&str, bool> = BTreeMap::new();
        for (token, change) in &z.log {
            if *token <= since {
                continue;
            }
            match change {
                Change::Saved(name) => latest.insert(name.as_str(), true),
                Change::Deleted(name) => latest.insert(name.as_str(), false),
            };
        }
        let mut saved = Vec::new();
        let mut deleted = Vec::new();
        for (name, is_saved) in latest {
            if is_saved {
                if let Some(r) = z.records.get(name) {
                    saved.push(r.clone());
                }
            } else {
                deleted.push(RecordID::new(zone, name));
            }
        }
        Ok(ChangeSet { token: z.token, full: false, saved, deleted })
    }

    fn zone(&self, name: &str) -> Result<&Zone, RecordError> {
        self.zones
            .get(name)
            .ok_or_else(|| RecordError::new(ErrorCode::ZoneNotFound, format!("no zone named {name:?}")))
    }

    fn zone_mut(&mut self, name: &str) -> Result<&mut Zone, RecordError> {
        self.zones
            .get_mut(name)
            .ok_or_else(|| RecordError::new(ErrorCode::ZoneNotFound, format!("no zone named {name:?}")))
    }

    fn check_save(&self, record: &Record, expected: Option<u64>) -> Result<(), RecordError> {
        if record.id.name.is_empty() || record.record_type.is_empty() {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "a record name and type must not be empty"));
        }
        let zone = self.zone(&record.id.zone)?;
        let current = zone.records.get(&record.id.name).map(|r| r.change_tag);
        match (expected, current) {
            (None, _) => {}
            (Some(0), None) => {}
            (Some(t), Some(c)) if t != 0 && t == c => {}
            (Some(t), None) if t != 0 => {
                return Err(RecordError::new(ErrorCode::UnknownItem, "the record to update does not exist"));
            }
            (Some(_), _) => {
                return Err(RecordError::new(ErrorCode::ServerRecordChanged, "the server holds a different version"));
            }
        }
        for value in record.fields.values() {
            if let Value::Ref(r) = value {
                if r.target.zone != record.id.zone {
                    return Err(RecordError::new(ErrorCode::InvalidArguments, "a reference must stay in its zone"));
                }
                if r.target.name != record.id.name && !zone.records.contains_key(&r.target.name) {
                    return Err(RecordError::new(
                        ErrorCode::ReferenceViolation,
                        format!("no record named {:?} to reference", r.target.name),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn trim(zone: &mut Zone, limit: usize) {
    while zone.log.len() > limit {
        let (token, _) = zone.log.remove(0);
        zone.floor = token;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(zone: &str, name: &str, kind: &str) -> Record {
        Record::new(RecordID::new(zone, name), kind)
    }

    fn zone_db() -> Database {
        let mut db = Database::new(Scope::Private);
        db.add_zone("notes").unwrap();
        db
    }

    #[test]
    fn save_sets_increasing_tags_and_clears_changed_keys() {
        let mut db = zone_db();
        let mut r = rec("notes", "a", "Note");
        r.set("title", Value::Str("one".into()));
        assert_eq!(r.changed_keys(), ["title".to_string()]);
        let t1 = db.save(r.clone()).unwrap();
        let t2 = db.save(r).unwrap();
        assert!(t2 > t1);
        let stored = db.fetch(&RecordID::new("notes", "a")).unwrap();
        assert_eq!(stored.change_tag, t2);
        assert!(stored.changed_keys().is_empty());
        assert_eq!(stored.get("title"), Some(&Value::Str("one".into())));
    }

    #[test]
    fn expected_tag_creates_or_replaces_only_when_it_matches() {
        let mut db = zone_db();
        let r = rec("notes", "a", "Note");
        let tag = db.save_expecting(r.clone(), Some(0)).unwrap();
        let again = db.save_expecting(r.clone(), Some(0)).unwrap_err();
        assert_eq!(again.code(), ErrorCode::ServerRecordChanged);
        let stale = db.save_expecting(r.clone(), Some(tag + 99)).unwrap_err();
        assert_eq!(stale.code(), ErrorCode::ServerRecordChanged);
        let ok = db.save_expecting(r.clone(), Some(tag)).unwrap();
        assert!(ok > tag);
        let missing = db.save_expecting(rec("notes", "b", "Note"), Some(5)).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::UnknownItem);
    }

    #[test]
    fn references_must_point_at_a_saved_record_in_the_same_zone() {
        let mut db = zone_db();
        let target = RecordID::new("notes", "folder");
        let mut child = rec("notes", "a", "Note");
        child.set("folder", Value::Ref(Reference { target: target.clone(), delete_self: false }));
        assert_eq!(db.save(child.clone()).unwrap_err().code(), ErrorCode::ReferenceViolation);
        db.save(rec("notes", "folder", "Folder")).unwrap();
        assert!(db.save(child).is_ok());
        let mut other = rec("notes", "x", "Note");
        other.set("folder", Value::Ref(Reference { target: RecordID::new(DEFAULT_ZONE, "folder"), delete_self: false }));
        assert_eq!(db.save(other).unwrap_err().code(), ErrorCode::InvalidArguments);
    }

    #[test]
    fn delete_cascades_only_through_delete_self_references() {
        let mut db = zone_db();
        db.save(rec("notes", "folder", "Folder")).unwrap();
        let mut owned = rec("notes", "owned", "Note");
        owned.set("folder", Value::Ref(Reference { target: RecordID::new("notes", "folder"), delete_self: true }));
        db.save(owned).unwrap();
        let mut linked = rec("notes", "linked", "Note");
        linked.set("folder", Value::Ref(Reference { target: RecordID::new("notes", "folder"), delete_self: false }));
        db.save(linked).unwrap();
        db.delete(&RecordID::new("notes", "folder")).unwrap();
        assert_eq!(db.fetch(&RecordID::new("notes", "owned")).unwrap_err().code(), ErrorCode::UnknownItem);
        assert!(db.fetch(&RecordID::new("notes", "linked")).is_ok());
    }

    #[test]
    fn query_filters_by_type_and_predicate_in_name_order() {
        let mut db = zone_db();
        for (name, kind, done) in [("b", "Task", true), ("a", "Task", false), ("c", "Note", true), ("d", "Task", true)] {
            let mut r = rec("notes", name, kind);
            r.set("done", Value::Bool(done));
            db.save(r).unwrap();
        }
        let done = Predicate::Equals("done".into(), Value::Bool(true));
        let names: Vec<&str> =
            db.query("notes", "Task", &done).unwrap().iter().map(|r| r.id.name.as_str()).collect();
        assert_eq!(names, ["b", "d"]);
        let not_done = Predicate::Not(Box::new(done));
        assert_eq!(db.query("notes", "Task", &not_done).unwrap().len(), 1);
        assert_eq!(db.query("notes", "Task", &Predicate::All).unwrap().len(), 3);
        assert_eq!(db.query("missing", "Task", &Predicate::All).unwrap_err().code(), ErrorCode::ZoneNotFound);
    }

    #[test]
    fn changes_since_returns_only_later_changes_once_in_latest_state() {
        let mut db = zone_db();
        db.save(rec("notes", "a", "Note")).unwrap();
        let seen = db.zone_token("notes").unwrap();
        db.save(rec("notes", "b", "Note")).unwrap();
        db.save(rec("notes", "b", "Note")).unwrap();
        db.save(rec("notes", "c", "Note")).unwrap();
        db.delete(&RecordID::new("notes", "c")).unwrap();
        let delta = db.changes_since("notes", seen).unwrap();
        assert!(!delta.full);
        let saved: Vec<&str> = delta.saved.iter().map(|r| r.id.name.as_str()).collect();
        assert_eq!(saved, ["b"]);
        assert_eq!(delta.deleted, vec![RecordID::new("notes", "c")]);
        assert_eq!(delta.token, db.zone_token("notes").unwrap());
        assert_eq!(db.changes_since("notes", delta.token).unwrap().saved.len(), 0);
    }

    #[test]
    fn a_token_older_than_the_log_expires_and_a_snapshot_is_complete() {
        let mut db = Database::with_log_limit(Scope::Public, 2);
        db.save(rec(DEFAULT_ZONE, "a", "Note")).unwrap();
        db.save(rec(DEFAULT_ZONE, "b", "Note")).unwrap();
        db.save(rec(DEFAULT_ZONE, "c", "Note")).unwrap();
        db.save(rec(DEFAULT_ZONE, "d", "Note")).unwrap();
        assert_eq!(db.changes_since(DEFAULT_ZONE, 0).unwrap_err().code(), ErrorCode::ChangeTokenExpired);
        let full = db.snapshot(DEFAULT_ZONE).unwrap();
        assert!(full.full);
        assert_eq!(full.saved.len(), 4);
        assert!(db.changes_since(DEFAULT_ZONE, 2).is_ok());
        assert_eq!(db.changes_since(DEFAULT_ZONE, 99).unwrap_err().code(), ErrorCode::InvalidArguments);
    }
}
