//! Sync queue, conflicts and error codes for the record database in `record_store`.
//!
//! `ErrorCode` mirrors the reference error codes (`CKError.Code`). `SyncEngine` keeps an app's pending saves and
//! deletes, coalesces repeated edits to one record, and pushes them to a `Database`. A save whose base change tag is
//! stale becomes a `Conflict` that holds the local and the server version; `resolve` applies one of three
//! `ConflictPolicy` values. `pull` returns the changes since the last token, and takes a full snapshot when the
//! token has expired.
//!
//! Which codes count as transient is our choice: the network, rate-limit, busy and service-unavailable codes, where
//! the same request may succeed later without the app changing anything. `push` keeps those operations queued until
//! `max_attempts` is reached.
//!
//! Gaps: no timer for retries (the caller calls `push` again, and `retry_after` is only carried on the error), no
//! account or network state, no push subscriptions, and conflict resolution offers three fixed policies instead of a
//! callback that merges the two versions.

use std::fmt;

use crate::record_store::{ChangeSet, Database, Record, RecordID};

/// Where a zone's records are kept: the in-memory `Database`, or a remote store reached over a transport (the cloud
/// zone in the SDK). `push` and `resolve` need only these three operations, so one sync queue serves both.
pub trait Remote {
    /// Saves `record` if the stored change tag is `expected` (`None` for a new record); returns the new tag.
    fn save_expecting(&mut self, record: Record, expected: Option<u64>) -> Result<u64, RecordError>;
    /// Deletes the record; returns the new tag of the zone.
    fn delete(&mut self, id: &RecordID) -> Result<u64, RecordError>;
    /// The stored version of the record.
    fn fetch(&self, id: &RecordID) -> Result<Record, RecordError>;
}

impl Remote for Database {
    fn save_expecting(&mut self, record: Record, expected: Option<u64>) -> Result<u64, RecordError> {
        Database::save_expecting(self, record, expected)
    }

    fn delete(&mut self, id: &RecordID) -> Result<u64, RecordError> {
        Database::delete(self, id)
    }

    fn fetch(&self, id: &RecordID) -> Result<Record, RecordError> {
        Database::fetch(self, id).cloned()
    }
}

/// The error codes a record operation can report (`CKError.Code`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    AccountTemporarilyUnavailable,
    AlreadyShared,
    AssetFileModified,
    AssetFileNotFound,
    AssetNotAvailable,
    BadContainer,
    BadDatabase,
    BatchRequestFailed,
    ChangeTokenExpired,
    ConstraintViolation,
    IncompatibleVersion,
    InternalError,
    InvalidArguments,
    LimitExceeded,
    ManagedAccountRestricted,
    MissingEntitlement,
    NetworkFailure,
    NetworkUnavailable,
    NotAuthenticated,
    OperationCancelled,
    PartialFailure,
    ParticipantMayNeedVerification,
    PermissionFailure,
    QuotaExceeded,
    ReferenceViolation,
    RequestRateLimited,
    ResultsTruncated,
    ServerRecordChanged,
    ServerRejectedRequest,
    ServerResponseLost,
    ServiceUnavailable,
    TooManyParticipants,
    UnknownItem,
    UserDeletedZone,
    ZoneBusy,
    ZoneNotFound,
}

impl ErrorCode {
    /// True for codes where the same request may succeed later without a change by the app.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            ErrorCode::AccountTemporarilyUnavailable
                | ErrorCode::NetworkFailure
                | ErrorCode::NetworkUnavailable
                | ErrorCode::RequestRateLimited
                | ErrorCode::ServerResponseLost
                | ErrorCode::ServiceUnavailable
                | ErrorCode::ZoneBusy
        )
    }
}

/// A failed record operation: its code, a message, and the wait the server asked for, when it asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordError {
    code: ErrorCode,
    message: String,
    retry_after: Option<u64>,
}

impl RecordError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        RecordError { code, message: message.into(), retry_after: None }
    }

    /// Sets `retryAfterSeconds`.
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retry_after(&self) -> Option<u64> {
        self.retry_after
    }

    pub fn is_transient(&self) -> bool {
        self.code.is_transient()
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for RecordError {}

/// What to do with a conflict between the local and the server version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Save the local version over the server version.
    KeepLocal,
    /// Drop the local version and keep the server version.
    KeepServer,
    /// Start from the server version and apply the keys the local version changed.
    Merge,
}

/// A local save the server rejected because its version is newer.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub id: RecordID,
    pub local: Record,
    pub server: Record,
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Save { record: Record, base_tag: Option<u64> },
    Delete { id: RecordID },
}

#[derive(Clone, Debug, PartialEq)]
struct Pending {
    op: Op,
    attempts: u32,
}

impl Pending {
    fn id(&self) -> &RecordID {
        match &self.op {
            Op::Save { record, .. } => &record.id,
            Op::Delete { id } => id,
        }
    }
}

/// Counts from one `push`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PushReport {
    pub saved: usize,
    pub deleted: usize,
    pub conflicts: usize,
    /// Operations that failed transiently and stay queued.
    pub deferred: usize,
    /// Operations that failed for good and were dropped.
    pub failed: Vec<RecordError>,
}

const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// A queue of local changes for one zone, with the last server token it pulled.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncEngine {
    zone: String,
    queue: Vec<Pending>,
    conflicts: Vec<Conflict>,
    server_token: u64,
    max_attempts: u32,
}

impl SyncEngine {
    pub fn new(zone: impl Into<String>) -> Self {
        SyncEngine {
            zone: zone.into(),
            queue: Vec::new(),
            conflicts: Vec::new(),
            server_token: 0,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }

    /// How many times a transient failure is retried before the operation is dropped (at least 1).
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }

    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// The zone token of the last pull; 0 before the first.
    pub fn server_token(&self) -> u64 {
        self.server_token
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    /// Queues a save. `base_tag` is the change tag the local copy was made from (`None` for a new record). A second
    /// save of the same record replaces the first and keeps its base tag, so the push still checks against the
    /// version the edits started from.
    pub fn enqueue_save(&mut self, record: Record, base_tag: Option<u64>) {
        let id = record.id.clone();
        match self.queue.iter().position(|p| p.id() == &id) {
            Some(i) => {
                let base = match &self.queue[i].op {
                    Op::Save { base_tag: earlier, .. } => *earlier,
                    Op::Delete { .. } => base_tag,
                };
                self.queue[i] = Pending { op: Op::Save { record, base_tag: base }, attempts: 0 };
            }
            None => self.queue.push(Pending { op: Op::Save { record, base_tag }, attempts: 0 }),
        }
    }

    /// Queues a delete, replacing any queued save of the same record.
    pub fn enqueue_delete(&mut self, id: RecordID) {
        let pending = Pending { op: Op::Delete { id: id.clone() }, attempts: 0 };
        match self.queue.iter().position(|p| p.id() == &id) {
            Some(i) => self.queue[i] = pending,
            None => self.queue.push(pending),
        }
    }

    /// Applies the queue to `db` in order. A stale save becomes a conflict; a transient failure stays queued until
    /// `max_attempts`; anything else is reported and dropped. A delete of a record that is already gone counts as
    /// done.
    pub fn push<R: Remote>(&mut self, db: &mut R) -> PushReport {
        let mut report = PushReport::default();
        let queue = std::mem::take(&mut self.queue);
        for mut pending in queue {
            let is_save = matches!(pending.op, Op::Save { .. });
            let result = match &pending.op {
                Op::Save { record, base_tag } => db.save_expecting(record.clone(), *base_tag).map(|_| ()),
                Op::Delete { id } => db.delete(id).map(|_| ()),
            };
            match result {
                Ok(()) if is_save => report.saved += 1,
                Ok(()) => report.deleted += 1,
                Err(e) if !is_save && e.code() == ErrorCode::UnknownItem => report.deleted += 1,
                Err(e) if e.code() == ErrorCode::ServerRecordChanged => {
                    if let Op::Save { record, .. } = &pending.op {
                        match db.fetch(&record.id) {
                            Ok(server) => {
                                self.conflicts.push(Conflict {
                                    id: record.id.clone(),
                                    local: record.clone(),
                                    server,
                                });
                                report.conflicts += 1;
                            }
                            Err(fetch_error) => report.failed.push(fetch_error),
                        }
                    }
                }
                Err(e) if e.is_transient() && pending.attempts + 1 < self.max_attempts => {
                    pending.attempts += 1;
                    self.queue.push(pending);
                    report.deferred += 1;
                }
                Err(e) => report.failed.push(e),
            }
        }
        report
    }

    /// Settles conflict `index` with `policy` and returns the change tag the record has afterwards. The conflict is
    /// removed only when the settlement succeeds; a newer server version makes it fail again.
    pub fn resolve<R: Remote>(&mut self, index: usize, policy: ConflictPolicy, db: &mut R) -> Result<u64, RecordError> {
        let conflict = match self.conflicts.get(index) {
            Some(c) => c.clone(),
            None => {
                return Err(RecordError::new(ErrorCode::InvalidArguments, format!("no conflict at index {index}")));
            }
        };
        let expected = Some(conflict.server.change_tag);
        let tag = match policy {
            ConflictPolicy::KeepServer => conflict.server.change_tag,
            ConflictPolicy::KeepLocal => db.save_expecting(conflict.local.clone(), expected)?,
            ConflictPolicy::Merge => {
                let mut merged = conflict.server.clone();
                for key in conflict.local.changed_keys() {
                    if let Some(value) = conflict.local.get(key) {
                        merged.set(key.clone(), value.clone());
                    }
                }
                db.save_expecting(merged, expected)?
            }
        };
        self.conflicts.remove(index);
        Ok(tag)
    }

    /// Returns the changes since the last pull, and moves the token forward. When the database has dropped the log
    /// past that token, returns a full snapshot instead.
    pub fn pull(&mut self, db: &Database) -> Result<ChangeSet, RecordError> {
        let changes = match db.changes_since(&self.zone, self.server_token) {
            Ok(c) => c,
            Err(e) if e.code() == ErrorCode::ChangeTokenExpired => db.snapshot(&self.zone)?,
            Err(e) => return Err(e),
        };
        self.server_token = changes.token;
        Ok(changes)
    }

    /// Pushes the queue, then pulls the changes the app must apply locally.
    pub fn sync(&mut self, db: &mut Database) -> Result<(PushReport, ChangeSet), RecordError> {
        let report = self.push(db);
        let changes = self.pull(db)?;
        Ok((report, changes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::{Scope, Value};

    fn note(name: &str, title: &str) -> Record {
        let mut r = Record::new(RecordID::new("notes", name), "Note");
        r.set("title", Value::Str(title.into()));
        r
    }

    fn db() -> Database {
        let mut db = Database::new(Scope::Private);
        db.add_zone("notes").unwrap();
        db
    }

    #[test]
    fn only_transient_codes_are_retried() {
        assert!(ErrorCode::NetworkUnavailable.is_transient());
        assert!(ErrorCode::RequestRateLimited.is_transient());
        assert!(!ErrorCode::InvalidArguments.is_transient());
        assert!(!ErrorCode::ServerRecordChanged.is_transient());
        let e = RecordError::new(ErrorCode::RequestRateLimited, "slow down").with_retry_after(30);
        assert_eq!(e.retry_after(), Some(30));
        assert_eq!(e.to_string(), "RequestRateLimited: slow down");
    }

    #[test]
    fn repeated_saves_of_one_record_coalesce_and_keep_the_first_base_tag() {
        let mut engine = SyncEngine::new("notes");
        engine.enqueue_save(note("a", "one"), Some(3));
        engine.enqueue_save(note("a", "two"), Some(9));
        assert_eq!(engine.pending(), 1);
        let mut d = db();
        d.save(note("a", "server")).unwrap();
        let report = engine.push(&mut d);
        assert_eq!(report.conflicts, 1);
        assert_eq!(engine.conflicts()[0].local.get("title"), Some(&Value::Str("two".into())));
    }

    #[test]
    fn a_delete_replaces_a_queued_save() {
        let mut engine = SyncEngine::new("notes");
        let mut d = db();
        d.save(note("a", "kept")).unwrap();
        engine.enqueue_save(note("a", "edit"), None);
        engine.enqueue_delete(RecordID::new("notes", "a"));
        assert_eq!(engine.pending(), 1);
        let report = engine.push(&mut d);
        assert_eq!(report.deleted, 1);
        assert_eq!(d.fetch(&RecordID::new("notes", "a")).unwrap_err().code(), ErrorCode::UnknownItem);
    }

    #[test]
    fn a_stale_save_becomes_a_conflict_and_keep_local_settles_it() {
        let mut engine = SyncEngine::new("notes");
        let mut d = db();
        let base = d.save(note("a", "base")).unwrap();
        d.save(note("a", "server edit")).unwrap();
        engine.enqueue_save(note("a", "local edit"), Some(base));
        let report = engine.push(&mut d);
        assert_eq!(report.conflicts, 1);
        assert_eq!(engine.pending(), 0);
        assert_eq!(d.fetch(&RecordID::new("notes", "a")).unwrap().get("title"), Some(&Value::Str("server edit".into())));
        engine.resolve(0, ConflictPolicy::KeepLocal, &mut d).unwrap();
        assert!(engine.conflicts().is_empty());
        assert_eq!(d.fetch(&RecordID::new("notes", "a")).unwrap().get("title"), Some(&Value::Str("local edit".into())));
    }

    #[test]
    fn merge_takes_the_server_version_and_the_local_changed_keys() {
        let mut engine = SyncEngine::new("notes");
        let mut d = db();
        let mut first = note("a", "t");
        first.set("body", Value::Str("old".into()));
        let base = d.save(first).unwrap();
        let mut server = d.fetch(&RecordID::new("notes", "a")).unwrap().clone();
        server.set("title", Value::Str("server title".into()));
        d.save_expecting(server, Some(base)).unwrap();
        let mut local_only_body = Record::new(RecordID::new("notes", "a"), "Note");
        local_only_body.set("body", Value::Str("local body".into()));
        engine.enqueue_save(local_only_body, Some(base));
        engine.push(&mut d);
        engine.resolve(0, ConflictPolicy::Merge, &mut d).unwrap();
        let merged = d.fetch(&RecordID::new("notes", "a")).unwrap();
        assert_eq!(merged.get("body"), Some(&Value::Str("local body".into())));
        assert_eq!(merged.get("title"), Some(&Value::Str("server title".into())));
    }

    #[test]
    fn keep_server_drops_the_local_version() {
        let mut engine = SyncEngine::new("notes");
        let mut d = db();
        let base = d.save(note("a", "base")).unwrap();
        d.save(note("a", "server")).unwrap();
        engine.enqueue_save(note("a", "local"), Some(base));
        engine.push(&mut d);
        let tag = engine.resolve(0, ConflictPolicy::KeepServer, &mut d).unwrap();
        assert_eq!(tag, d.fetch(&RecordID::new("notes", "a")).unwrap().change_tag);
        assert_eq!(d.fetch(&RecordID::new("notes", "a")).unwrap().get("title"), Some(&Value::Str("server".into())));
        assert_eq!(engine.resolve(0, ConflictPolicy::KeepServer, &mut d).unwrap_err().code(), ErrorCode::InvalidArguments);
    }

    #[test]
    fn a_permanent_failure_is_reported_and_dropped() {
        let mut engine = SyncEngine::new("missing");
        engine.enqueue_save(Record::new(RecordID::new("missing", "a"), "Note"), None);
        let mut d = db();
        let report = engine.push(&mut d);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].code(), ErrorCode::ZoneNotFound);
        assert_eq!(engine.pending(), 0);
    }

    #[test]
    fn pull_returns_only_new_changes_and_falls_back_to_a_snapshot() {
        let mut d = Database::with_log_limit(Scope::Public, 2);
        d.add_zone("notes").unwrap();
        let mut engine = SyncEngine::new("notes");
        d.save(note("a", "1")).unwrap();
        let first = engine.pull(&d).unwrap();
        assert_eq!(first.saved.len(), 1);
        assert_eq!(engine.server_token(), first.token);
        d.save(note("b", "2")).unwrap();
        d.save(note("c", "3")).unwrap();
        d.save(note("d", "4")).unwrap();
        let snap = engine.pull(&d).unwrap();
        assert!(snap.full);
        assert_eq!(snap.saved.len(), 4);
        assert_eq!(engine.server_token(), d.zone_token("notes").unwrap());
        let quiet = engine.pull(&d).unwrap();
        assert!(!quiet.full && quiet.saved.is_empty());
    }

    #[test]
    fn sync_pushes_then_pulls_so_the_local_side_sees_its_own_saves() {
        let mut engine = SyncEngine::new("notes");
        let mut d = db();
        engine.enqueue_save(note("a", "hello"), None);
        let (report, changes) = engine.sync(&mut d).unwrap();
        assert_eq!(report.saved, 1);
        assert_eq!(changes.saved.len(), 1);
        assert_eq!(changes.saved[0].id.name, "a");
    }

    /// A remote that is not the local database: it counts the saves it receives and delegates to a database.
    struct CountingRemote {
        inner: Database,
        saves: usize,
    }

    impl Remote for CountingRemote {
        fn save_expecting(&mut self, record: Record, expected: Option<u64>) -> Result<u64, RecordError> {
            self.saves += 1;
            self.inner.save_expecting(record, expected)
        }

        fn delete(&mut self, id: &RecordID) -> Result<u64, RecordError> {
            self.inner.delete(id)
        }

        fn fetch(&self, id: &RecordID) -> Result<Record, RecordError> {
            self.inner.fetch(id).cloned()
        }
    }

    #[test]
    fn a_remote_store_takes_the_same_queue_and_conflicts() {
        let mut remote = CountingRemote { inner: db(), saves: 0 };
        let mut engine = SyncEngine::new("notes");
        engine.enqueue_save(note("a", "first"), None);
        assert_eq!(engine.push(&mut remote).saved, 1);
        assert_eq!(remote.saves, 1);
        // A stale base tag is a conflict against the remote's version, as with the local database.
        engine.enqueue_save(note("a", "second"), Some(0));
        let report = engine.push(&mut remote);
        assert_eq!(report.conflicts, 1);
        assert_eq!(engine.conflicts()[0].server.get("title"), Some(&Value::Str("first".into())));
        assert_eq!(engine.resolve(0, ConflictPolicy::KeepLocal, &mut remote).unwrap(), 2);
        assert_eq!(remote.inner.fetch(&RecordID::new("notes", "a")).unwrap().get("title"), Some(&Value::Str("second".into())));
    }
}
