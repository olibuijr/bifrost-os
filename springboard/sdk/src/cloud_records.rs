//! Record sync against the BifrOSt Cloud, for apps: one zone of records kept in the cloud's `records` table through the
//! generic data API (`/data/records`). [`CloudZone`] implements `record_sync::Remote`, so the same `SyncEngine` queue
//! and conflict model that works on the local `Database` works on the cloud zone unchanged.
//!
//! Privacy: a zone can be opened only with explicit [`Consent`]. The cloud stores the record name, its type, its change
//! tag and a deleted flag; the field values travel only inside `fields_enc`, which the caller seals through a
//! [`Sealer`] before it leaves the device. The cloud cannot open what it holds. Until an app has an account key to
//! seal with, it has no sealer and cannot sync.
//!
//! Limits, stated plainly: the data API has no compare-and-swap, so `save_expecting` reads the row, compares its change
//! tag, then writes, in two requests; two devices that write in between can both pass the check. Only text, integer,
//! real, boolean and date fields sync; references and assets are refused before anything is sent.
//!
//! Everything that builds a request or reads an answer is pure, and the transport is a closure, so the walk is tested
//! with canned answers and no socket. Each call blocks; run it on a background thread.

use crate::cloud_account::{checked_token, endpoint, failure, CloudError, Reply, Transport};
use crate::url::URLRequest;
use bifrost_core::json::{self, Json};
use bifrost_ui::record_store::{Record, RecordID, Value};
use bifrost_ui::record_sync::{ErrorCode, RecordError, Remote};
use std::cell::RefCell;

/// The table the records live in (see `bifrost-cloud` `records`).
pub const TABLE: &str = "records";
/// The most rows one list request asks for (the service's own cap).
pub const PAGE: usize = 200;
/// The largest answer read for one request.
pub const MAX_ANSWER_BYTES: usize = 4 * 1024 * 1024;

/// The user's explicit consent to sync a zone. Off until the user turns it on; opening a zone needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Consent(bool);

impl Consent {
    pub fn given(granted: bool) -> Consent {
        Consent(granted)
    }

    pub fn is_given(self) -> bool {
        self.0
    }
}

/// Seals and opens the field values of a record. The app supplies it from its account key; the cloud never sees the
/// plain text.
pub trait Sealer {
    fn seal(&self, plain: &[u8]) -> Result<String, String>;
    fn open(&self, sealed: &str) -> Result<Vec<u8>, String>;
}

/// Why a zone could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneError {
    /// The user has not consented to sync this zone; nothing is sent.
    NoConsent,
    /// The base address or the session is not usable.
    Cloud(CloudError),
}

/// One row as the service returns it, before its sealed fields are opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub record_type: String,
    pub change_tag: u64,
    /// The sealed fields; empty for a deleted record.
    pub sealed: String,
    pub deleted: bool,
}

fn malformed(what: &str) -> RecordError {
    RecordError::new(ErrorCode::ServerRejectedRequest, format!("the cloud record answer is malformed: {what}"))
}

/// Percent-encodes a query value: everything but letters, digits and `-_.~` is escaped.
fn query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn with_session(mut req: URLRequest, session_token: &str) -> Result<URLRequest, CloudError> {
    let token = checked_token(session_token)?;
    req.set_header("Authorization", &format!("Bearer {token}"));
    req.set_header("Accept", "application/json");
    Ok(req)
}

fn json_request(mut req: URLRequest, method: &str, body: &Json) -> URLRequest {
    req.method = method.into();
    req.body = Some(body.encode().into_bytes());
    req.set_header("Content-Type", "application/json");
    req
}

/// `GET {base}/data/records?zone_id=Z&offset=N&limit=PAGE`: one page of the zone's rows.
pub fn list_request(base_url: &str, session_token: &str, zone: &str, offset: usize) -> Result<URLRequest, CloudError> {
    let path = format!("data/{TABLE}?zone_id={}&limit={PAGE}&offset={offset}", query_value(zone));
    with_session(URLRequest::new(&endpoint(base_url, &path)?), session_token)
}

/// `GET {base}/data/records?zone_id=Z&name=N&limit=1`: the row of one record.
pub fn fetch_request(base_url: &str, session_token: &str, id: &RecordID) -> Result<URLRequest, CloudError> {
    let path = format!("data/{TABLE}?zone_id={}&name={}&limit=1", query_value(&id.zone), query_value(&id.name));
    with_session(URLRequest::new(&endpoint(base_url, &path)?), session_token)
}

/// The write of one row: `POST` creates it (the body carries `zone_id` and `name`), `PATCH` updates the row at its
/// primary key (the key goes in the query, and the body never carries it).
fn write_request(base_url: &str, session_token: &str, zone: &str, name: &str, create: bool, row: Json) -> Result<URLRequest, CloudError> {
    if create {
        let mut obj = match row {
            Json::Obj(v) => v,
            _ => Vec::new(),
        };
        obj.insert(0, ("zone_id".into(), Json::Str(zone.into())));
        obj.insert(1, ("name".into(), Json::Str(name.into())));
        let req = URLRequest::new(&endpoint(base_url, &format!("data/{TABLE}"))?);
        return with_session(json_request(req, "POST", &Json::Obj(obj)), session_token);
    }
    let path = format!("data/{TABLE}/item?zone_id={}&name={}", query_value(zone), query_value(name));
    let req = URLRequest::new(&endpoint(base_url, &path)?);
    with_session(json_request(req, "PATCH", &row), session_token)
}

/// Reads one page of rows: `200` with `{"rows": [...]}`.
pub fn parse_rows(status: u16, body: &str) -> Result<Vec<Row>, RecordError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(status_error(status, body));
    }
    let doc = json::parse(body).map_err(|e| malformed(&e))?;
    let rows = doc.get("rows").and_then(Json::as_array).ok_or_else(|| malformed("no rows array"))?;
    rows.iter().map(row_from_json).collect()
}

fn row_from_json(j: &Json) -> Result<Row, RecordError> {
    let text = |k: &str| j.str_field(k).ok_or_else(|| malformed(&format!("no {k}")));
    let number = |k: &str| j.get(k).and_then(Json::number).map(|n| n as u64).ok_or_else(|| malformed(&format!("no {k}")));
    Ok(Row {
        name: text("name")?,
        record_type: text("record_type")?,
        change_tag: number("change_tag")?,
        sealed: j.str_field("fields_enc").unwrap_or_default(),
        deleted: number("deleted")? != 0,
    })
}

/// Maps a refused request to the record error codes the sync queue understands.
fn status_error(status: u16, body: &str) -> RecordError {
    let code = match status {
        401 => ErrorCode::NotAuthenticated,
        403 => ErrorCode::PermissionFailure,
        404 => ErrorCode::UnknownItem,
        409 => ErrorCode::ServerRecordChanged,
        429 => ErrorCode::RequestRateLimited,
        500..=599 => ErrorCode::ServiceUnavailable,
        _ => ErrorCode::ServerRejectedRequest,
    };
    let why = match failure(status, body) {
        CloudError::Rejected { code, .. } if !code.is_empty() => code,
        other => other.to_string(),
    };
    RecordError::new(code, why)
}

fn value_json(v: &Value) -> Result<Json, String> {
    let tagged = |t: &str, v: Json| Json::Obj(vec![("t".into(), Json::Str(t.into())), ("v".into(), v)]);
    Ok(match v {
        Value::Str(s) => tagged("s", Json::Str(s.clone())),
        // integers and dates travel as text so no digit is lost to the f64 of the JSON numbers
        Value::Int(n) => tagged("i", Json::Str(n.to_string())),
        Value::Real(r) => tagged("r", Json::Num(*r)),
        Value::Bool(b) => tagged("b", Json::Bool(*b)),
        Value::Date(d) => tagged("d", Json::Str(d.to_string())),
        Value::Ref(_) | Value::Asset(_) => return Err("references and assets do not sync yet".into()),
    })
}

fn value_from(j: &Json) -> Option<Value> {
    let t = j.str_field("t")?;
    let v = j.get("v")?;
    match t.as_str() {
        "s" => v.as_str().map(|s| Value::Str(s.to_string())),
        "i" => v.as_str()?.parse::<i64>().ok().map(Value::Int),
        "r" => v.number().map(Value::Real),
        "b" => match v {
            Json::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        },
        "d" => v.as_str()?.parse::<u64>().ok().map(Value::Date),
        _ => None,
    }
}

/// The plain fields of a record as the JSON object that gets sealed.
pub fn fields_text(record: &Record) -> Result<String, String> {
    let mut obj = Vec::with_capacity(record.fields.len());
    for (k, v) in &record.fields {
        obj.push((k.clone(), value_json(v)?));
    }
    Ok(Json::Obj(obj).encode())
}

/// Rebuilds a record from its opened fields and the row's type and change tag.
pub fn record_from_fields(id: RecordID, record_type: &str, change_tag: u64, text: &str) -> Result<Record, String> {
    let Json::Obj(pairs) = json::parse(text)? else {
        return Err("the record fields are not an object".into());
    };
    let mut record = Record::new(id, record_type);
    for (k, v) in pairs {
        let value = value_from(&v).ok_or_else(|| format!("field {k} has no known value"))?;
        record.set(k, value);
    }
    record.change_tag = change_tag;
    Ok(record)
}

/// The row body for a record: the sealed fields, the change tag and the deleted flag. The plain `fields` column holds
/// an empty object, so the cloud sees no values.
fn row_body(record_type: &str, sealed: &str, change_tag: u64, deleted: bool) -> Json {
    Json::Obj(vec![
        ("record_type".into(), Json::Str(record_type.into())),
        ("fields".into(), Json::Obj(Vec::new())),
        ("fields_enc".into(), Json::Str(sealed.into())),
        ("change_tag".into(), Json::Num(change_tag as f64)),
        ("deleted".into(), Json::Num(if deleted { 1.0 } else { 0.0 })),
    ])
}

/// One zone of the account's cloud records, opened with consent. It implements [`Remote`], so a `SyncEngine` pushes
/// to it and resolves its conflicts against it.
pub struct CloudZone<'a> {
    base_url: String,
    session_token: String,
    zone: String,
    sealer: &'a dyn Sealer,
    send: RefCell<&'a mut Transport<'a>>,
}

impl<'a> CloudZone<'a> {
    /// Opens `zone` for sync. Refused without consent, and refused when the base address or the session is unusable;
    /// no request is made either way.
    pub fn open(base_url: &str, session_token: &str, zone: &str, consent: Consent, sealer: &'a dyn Sealer, send: &'a mut Transport<'a>) -> Result<CloudZone<'a>, ZoneError> {
        if !consent.is_given() {
            return Err(ZoneError::NoConsent);
        }
        endpoint(base_url, TABLE).map_err(ZoneError::Cloud)?;
        checked_token(session_token).map_err(ZoneError::Cloud)?;
        Ok(CloudZone {
            base_url: base_url.to_string(),
            session_token: session_token.to_string(),
            zone: zone.to_string(),
            sealer,
            send: RefCell::new(send),
        })
    }

    fn call(&self, req: &URLRequest) -> Result<Reply, RecordError> {
        let mut send = self.send.borrow_mut();
        (*send)(req).map_err(|e| RecordError::new(ErrorCode::NetworkUnavailable, e))
    }

    /// The row of `id`, or `None` when the cloud has none.
    fn row(&self, id: &RecordID) -> Result<Option<Row>, RecordError> {
        let req = fetch_request(&self.base_url, &self.session_token, id).map_err(cloud_to_record)?;
        let reply = self.call(&req)?;
        Ok(parse_rows(reply.status, &reply.body)?.into_iter().find(|r| r.name == id.name))
    }

    /// Every live record of the zone, opened with the sealer. Deleted rows are skipped.
    pub fn pull_all(&self) -> Result<Vec<Record>, RecordError> {
        let mut out = Vec::new();
        let mut offset = 0;
        loop {
            let req = list_request(&self.base_url, &self.session_token, &self.zone, offset).map_err(cloud_to_record)?;
            let reply = self.call(&req)?;
            let rows = parse_rows(reply.status, &reply.body)?;
            let n = rows.len();
            for row in rows.into_iter().filter(|r| !r.deleted) {
                out.push(self.open_row(&row)?);
            }
            if n < PAGE {
                return Ok(out);
            }
            offset += n;
        }
    }

    fn open_row(&self, row: &Row) -> Result<Record, RecordError> {
        let plain = self.sealer.open(&row.sealed).map_err(|e| RecordError::new(ErrorCode::InternalError, e))?;
        let text = String::from_utf8(plain).map_err(|_| RecordError::new(ErrorCode::InternalError, "the sealed fields are not text"))?;
        record_from_fields(RecordID::new(self.zone.clone(), row.name.clone()), &row.record_type, row.change_tag, &text)
            .map_err(|e| RecordError::new(ErrorCode::InternalError, e))
    }

    fn write(&self, name: &str, create: bool, row: Json) -> Result<(), RecordError> {
        let req = write_request(&self.base_url, &self.session_token, &self.zone, name, create, row).map_err(cloud_to_record)?;
        let reply = self.call(&req)?;
        if reply.status == 200 || reply.status == 201 || reply.status == 204 {
            Ok(())
        } else {
            Err(status_error(reply.status, &reply.body))
        }
    }
}

fn cloud_to_record(e: CloudError) -> RecordError {
    RecordError::new(ErrorCode::InvalidArguments, e.to_string())
}

impl Remote for CloudZone<'_> {
    fn save_expecting(&mut self, record: Record, expected: Option<u64>) -> Result<u64, RecordError> {
        let plain = fields_text(&record).map_err(|e| RecordError::new(ErrorCode::InvalidArguments, e))?;
        let sealed = self.sealer.seal(plain.as_bytes()).map_err(|e| RecordError::new(ErrorCode::InternalError, e))?;
        let current = self.row(&record.id)?;
        let (create, base) = match (expected, &current) {
            (None, None) => (true, 0),
            (None, Some(row)) if row.deleted => (false, row.change_tag),
            (None, Some(_)) => return Err(RecordError::new(ErrorCode::ServerRecordChanged, "the record exists on the cloud")),
            (Some(_), None) => return Err(RecordError::new(ErrorCode::UnknownItem, "the record is not on the cloud")),
            (Some(e), Some(row)) if row.deleted || row.change_tag != e => {
                return Err(RecordError::new(ErrorCode::ServerRecordChanged, "the record changed on the cloud"));
            }
            (Some(_), Some(row)) => (false, row.change_tag),
        };
        let tag = base + 1;
        self.write(&record.id.name, create, row_body(&record.record_type, &sealed, tag, false))?;
        Ok(tag)
    }

    fn delete(&mut self, id: &RecordID) -> Result<u64, RecordError> {
        let row = self.row(id)?.ok_or_else(|| RecordError::new(ErrorCode::UnknownItem, "the record is not on the cloud"))?;
        let tag = row.change_tag + 1;
        self.write(&id.name, false, row_body(&row.record_type, "", tag, true))?;
        Ok(tag)
    }

    fn fetch(&self, id: &RecordID) -> Result<Record, RecordError> {
        let row = self.row(id)?.ok_or_else(|| RecordError::new(ErrorCode::UnknownItem, "the record is not on the cloud"))?;
        if row.deleted {
            return Err(RecordError::new(ErrorCode::UnknownItem, "the record was deleted on the cloud"));
        }
        self.open_row(&row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::record_store::{Database, Reference, Scope};
    use std::collections::BTreeMap;
    use bifrost_ui::record_sync::{ConflictPolicy, SyncEngine};

    /// A sealer for tests: the sealed text is the plain text in base64, so a test can check the cloud never sees the
    /// values in clear.
    struct Plain;

    impl Sealer for Plain {
        fn seal(&self, plain: &[u8]) -> Result<String, String> {
            Ok(bifrost_core::plist::base64_encode(plain))
        }

        fn open(&self, sealed: &str) -> Result<Vec<u8>, String> {
            bifrost_core::plist::base64_decode(sealed).ok_or_else(|| "not base64".to_string())
        }
    }

    const BASE: &str = "https://bifrost.example/api/cloud/v1";
    const TOKEN: &str = "abc123";

    /// A fake cloud: one table of rows keyed by name, answering the data API the way the service does.
    #[derive(Default)]
    struct Fake {
        rows: BTreeMap<String, Json>,
        requests: Vec<(String, String)>,
    }

    impl Fake {
        fn answer(&mut self, req: &URLRequest) -> Result<Reply, String> {
            let body_json = req.body.as_ref().map(|b| json::parse(&String::from_utf8_lossy(b)).unwrap());
            self.requests.push((req.method.clone(), req.url.clone()));
            let query = req.url.split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
            let name = query.split('&').find_map(|kv| kv.strip_prefix("name=")).map(str::to_string);
            let ok = |status: u16, body: String| Ok(Reply { status, set_cookie: None, body });
            match (req.method.as_str(), name) {
                ("GET", None) => {
                    let rows: Vec<Json> = self.rows.values().cloned().collect();
                    ok(200, Json::Obj(vec![("rows".into(), Json::Arr(rows))]).encode())
                }
                ("GET", Some(n)) => {
                    let rows: Vec<Json> = self.rows.get(&n).cloned().into_iter().collect();
                    ok(200, Json::Obj(vec![("rows".into(), Json::Arr(rows))]).encode())
                }
                ("POST", _) => {
                    let mut obj = match body_json.unwrap() {
                        Json::Obj(v) => v,
                        _ => panic!("body"),
                    };
                    let name = obj.iter().find(|(k, _)| k.as_str() == "name").and_then(|(_, v)| v.as_str().map(str::to_string)).unwrap();
                    obj.retain(|(k, _)| k.as_str() != "zone_id");
                    self.rows.insert(name, Json::Obj(obj));
                    ok(201, "{}".into())
                }
                ("PATCH", Some(n)) => {
                    let body = body_json.unwrap();
                    let stored = self.rows.get_mut(&n).expect("patch of a missing row");
                    if let (Json::Obj(old), Json::Obj(new)) = (stored, body) {
                        for (k, v) in new {
                            match old.iter_mut().find(|(ok, _)| *ok == k) {
                                Some(slot) => slot.1 = v,
                                None => old.push((k, v)),
                            }
                        }
                    }
                    ok(200, "{}".into())
                }
                _ => ok(400, "{}".into()),
            }
        }
    }

    fn row_tag(fake: &RefCell<Fake>, name: &str) -> Option<u64> {
        fake.borrow().rows.get(name).and_then(|r| r.get("change_tag")).and_then(Json::number).map(|n| n as u64)
    }

    fn reminder(name: &str, title: &str) -> Record {
        let mut r = Record::new(RecordID::new("reminders", name), "Reminder");
        r.set("title", Value::Str(title.into()));
        r.set("due", Value::Date(1_800_000_000));
        r.set("priority", Value::Int(2));
        r.set("done", Value::Bool(false));
        r
    }

    #[test]
    fn nothing_is_sent_without_consent() {
        let fake = RefCell::new(Fake::default());
        let mut send = |req: &URLRequest| fake.borrow_mut().answer(req);
        let sealer = Plain;
        let r = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(false), &sealer, &mut send);
        assert_eq!(r.err(), Some(ZoneError::NoConsent));
        assert!(fake.borrow().requests.is_empty());
    }

    #[test]
    fn a_bad_base_or_session_is_refused_before_any_request() {
        let fake = RefCell::new(Fake::default());
        let mut send = |req: &URLRequest| fake.borrow_mut().answer(req);
        let sealer = Plain;
        assert!(matches!(CloudZone::open("http://x", TOKEN, "reminders", Consent::given(true), &sealer, &mut send), Err(ZoneError::Cloud(_))));
        assert!(matches!(CloudZone::open(BASE, "bad token", "reminders", Consent::given(true), &sealer, &mut send), Err(ZoneError::Cloud(_))));
        assert!(fake.borrow().requests.is_empty());
    }

    #[test]
    fn values_round_trip_through_the_fields_text() {
        let r = reminder("r1", "Milk");
        let text = fields_text(&r).unwrap();
        let back = record_from_fields(r.id.clone(), "Reminder", 7, &text).unwrap();
        assert_eq!(back.get("title"), r.get("title"));
        assert_eq!(back.get("due"), Some(&Value::Date(1_800_000_000)));
        assert_eq!(back.get("priority"), Some(&Value::Int(2)));
        assert_eq!(back.change_tag, 7);
    }

    #[test]
    fn references_and_assets_are_refused() {
        let mut r = Record::new(RecordID::new("reminders", "r1"), "Reminder");
        r.set("parent", Value::Ref(Reference { target: RecordID::new("reminders", "p"), delete_self: false }));
        assert!(fields_text(&r).is_err());
    }

    #[test]
    fn save_then_stale_save_is_a_conflict_on_the_cloud() {
        let fake = RefCell::new(Fake::default());
        let sealer = Plain;
        let mut send = |req: &URLRequest| fake.borrow_mut().answer(req);
        let mut engine = SyncEngine::new("reminders");
        {
            let mut zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut send).unwrap();
            engine.enqueue_save(reminder("r1", "Milk"), None);
            assert_eq!(engine.push(&mut zone).saved, 1);
        }
        assert_eq!(row_tag(&fake, "r1"), Some(1));
        // the cloud holds the sealed text, never the values
        assert!(!fake.borrow().rows["r1"].get("fields_enc").and_then(Json::as_str).unwrap().contains("Milk"));
        {
            // a second device edits the cloud copy from tag 1
            let mut other = |req: &URLRequest| fake.borrow_mut().answer(req);
            let mut zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut other).unwrap();
            let mut second = SyncEngine::new("reminders");
            second.enqueue_save(reminder("r1", "Bread"), Some(1));
            assert_eq!(second.push(&mut zone).saved, 1);
        }
        {
            // the first device saves against its old tag: a conflict with the cloud's version
            let mut zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut send).unwrap();
            engine.enqueue_save(reminder("r1", "Eggs"), Some(1));
            let report = engine.push(&mut zone);
            assert_eq!(report.conflicts, 1);
            assert_eq!(engine.conflicts()[0].server.get("title"), Some(&Value::Str("Bread".into())));
            // keeping the local version writes it over the cloud's copy with the next tag
            assert_eq!(engine.resolve(0, ConflictPolicy::KeepLocal, &mut zone).unwrap(), 3);
        }
        assert_eq!(row_tag(&fake, "r1"), Some(3));
    }

    #[test]
    fn delete_marks_the_row_and_pull_skips_it() {
        let fake = RefCell::new(Fake::default());
        let sealer = Plain;
        let mut send = |req: &URLRequest| fake.borrow_mut().answer(req);
        {
            let mut zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut send).unwrap();
            zone.save_expecting(reminder("r1", "Milk"), None).unwrap();
            zone.save_expecting(reminder("r2", "Tea"), None).unwrap();
            zone.delete(&RecordID::new("reminders", "r1")).unwrap();
        }
        assert_eq!(row_tag(&fake, "r1"), Some(2));
        let zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut send).unwrap();
        let live = zone.pull_all().unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id.name, "r2");
        assert_eq!(live[0].get("title"), Some(&Value::Str("Tea".into())));
    }

    #[test]
    fn a_fetch_of_a_missing_record_is_unknown() {
        let fake = RefCell::new(Fake::default());
        let sealer = Plain;
        let mut send = |req: &URLRequest| fake.borrow_mut().answer(req);
        let zone = CloudZone::open(BASE, TOKEN, "reminders", Consent::given(true), &sealer, &mut send).unwrap();
        assert_eq!(zone.fetch(&RecordID::new("reminders", "nope")).unwrap_err().code(), ErrorCode::UnknownItem);
    }

    #[test]
    fn the_local_database_still_takes_the_same_queue() {
        let mut db = Database::new(Scope::Private);
        db.add_zone("reminders").unwrap();
        let mut engine = SyncEngine::new("reminders");
        engine.enqueue_save(reminder("r1", "Milk"), None);
        assert_eq!(engine.push(&mut db).saved, 1);
    }
}
