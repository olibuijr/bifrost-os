//! Find My against the BifrOSt Cloud HTTP API: the requests an app sends to list the account's devices, to queue a
//! command for one of them (locate, play sound, erase), and to report this device's battery and position, plus the
//! answers those requests read back. The routes are `GET {base}/findmy/devices`, `POST {base}/findmy/devices/{id}/commands`
//! and `PUT {base}/findmy/devices/self/report`, and every request carries the account's `bc_session` cookie, as the
//! sign-in module does.
//!
//! Everything here is pure: requests are built and answers read with no socket, so the walk is tested with canned
//! answers. Each call blocks; run it on a background thread. The service side of these routes lives in bifrost-cloud.

use crate::cloud_account::{checked_token, endpoint, failure, CloudError, MAX_ANSWER_BYTES};
use crate::url::URLRequest;
use bifrost_core::json::{self, Json};

/// A command an app can queue for a device. `Erase` is destructive and needs the person's confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Locate,
    PlaySound,
    Erase,
}

impl CommandKind {
    /// The name the service uses for the command on the wire.
    pub fn wire(self) -> &'static str {
        match self {
            CommandKind::Locate => "locate",
            CommandKind::PlaySound => "play_sound",
            CommandKind::Erase => "erase",
        }
    }
}

/// A position a device reported: degrees, metres of accuracy when known, and the time it was taken (Unix seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudFix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_m: Option<f64>,
    pub recorded_at: i64,
}

/// One device of the account as the service lists it. The session id is never sent to the app.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub last_seen: i64,
    pub battery_percent: Option<u8>,
    pub is_charging: Option<bool>,
    pub fix: Option<CloudFix>,
    /// The sealed position the cloud holds for this device, when it has one. The cloud cannot open it.
    pub sealed_location: Option<SealedBlob>,
}

/// The answer to a device list: every device of the account, and the id of the device that asked when it is one.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceList {
    pub devices: Vec<CloudDevice>,
    pub this_device: Option<String>,
}

fn malformed(what: &str) -> CloudError {
    CloudError::MalformedAnswer(what.to_string())
}

/// Sends the device's bearer token (`Authorization: Bearer`), the form a device session takes.
fn with_session(mut req: URLRequest, session_token: &str) -> Result<URLRequest, CloudError> {
    let token = checked_token(session_token)?;
    req.set_header("Authorization", &format!("Bearer {token}"));
    req.set_header("Accept", "application/json");
    Ok(req)
}

/// Device ids are the service's hex ids; anything else is refused before a request is built.
fn checked_id(id: &str) -> Result<&str, CloudError> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()) { Ok(id) } else { Err(CloudError::Rejected { status: 400, code: "bad_device_id".into() }) }
}

/// The `GET {base}/findmy/devices` request for the session in `session_token`.
pub fn devices_request(base_url: &str, session_token: &str) -> Result<URLRequest, CloudError> {
    with_session(URLRequest::new(&endpoint(base_url, "findmy/devices")?), session_token)
}

/// Reads the answer to a device list: `200` with `{"devices": [...], "this_device": id}`. Each device needs an `id`;
/// a `location` object, when present, needs finite coordinates in range.
pub fn parse_devices(status: u16, body: &str) -> Result<DeviceList, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let items = parsed.get("devices").and_then(Json::as_array).ok_or_else(|| malformed("no devices"))?;
    let devices = items.iter().map(read_device).collect::<Result<Vec<_>, _>>()?;
    let this_device = parsed.get("this_device").and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(DeviceList { devices, this_device })
}

fn whole(j: &Json, key: &str) -> Option<i64> {
    j.get(key).and_then(Json::number).filter(|n| n.is_finite() && n.fract() == 0.0).map(|n| n as i64)
}

fn read_device(j: &Json) -> Result<CloudDevice, CloudError> {
    let id = j.str_field("id").filter(|s| !s.is_empty()).ok_or_else(|| malformed("device has no id"))?;
    let battery_percent = whole(j, "battery_percent").filter(|n| (0..=100).contains(n)).map(|n| n as u8);
    let is_charging = match j.get("is_charging") {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    };
    let fix = match j.get("location") {
        None | Some(Json::Null) => None,
        Some(loc) => Some(read_fix(loc)?),
    };
    Ok(CloudDevice {
        id,
        name: j.str_field("name").unwrap_or_default(),
        model: j.str_field("model").unwrap_or_default(),
        last_seen: whole(j, "last_seen").unwrap_or(0),
        battery_percent,
        is_charging,
        fix,
        sealed_location: sealed_blob(j, "location_sealed", "location_enc", "location_updated"),
    })
}

fn read_fix(j: &Json) -> Result<CloudFix, CloudError> {
    let coord = |key: &str, limit: f64| -> Result<f64, CloudError> {
        j.get(key)
            .and_then(Json::number)
            .filter(|n| n.is_finite() && n.abs() <= limit)
            .ok_or_else(|| malformed(&format!("location has no valid {key}")))
    };
    Ok(CloudFix {
        latitude: coord("latitude", 90.0)?,
        longitude: coord("longitude", 180.0)?,
        accuracy_m: j.get("accuracy_m").and_then(Json::number).filter(|n| n.is_finite() && *n >= 0.0),
        recorded_at: whole(j, "recorded_at").unwrap_or(0),
    })
}

/// The `POST {base}/findmy/devices/{id}/commands` request that queues `kind` for the device. Erase is refused unless
/// `confirmed` is true: the app must ask the person first, and the service checks the same flag.
pub fn command_request(base_url: &str, session_token: &str, device_id: &str, kind: CommandKind, confirmed: bool) -> Result<URLRequest, CloudError> {
    let id = checked_id(device_id)?;
    if kind == CommandKind::Erase && !confirmed {
        return Err(CloudError::Rejected { status: 400, code: "erase_needs_confirmation".into() });
    }
    let body = Json::Obj(vec![("kind".into(), Json::Str(kind.wire().into())), ("confirmed".into(), Json::Bool(confirmed))]);
    let mut req = URLRequest::post(&endpoint(base_url, &format!("findmy/devices/{id}/commands"))?, body.encode().into_bytes());
    req.set_header("Content-Type", "application/json");
    with_session(req, session_token)
}

/// Reads the answer to a command request: `201` with `{"command": {"id": ...}}`, which gives the queued command's id.
pub fn parse_command(status: u16, body: &str) -> Result<String, CloudError> {
    if status != 201 {
        return Err(failure(status, body));
    }
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    parsed
        .get("command")
        .and_then(|c| c.str_field("id"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| malformed("no command id"))
}

/// The `PUT {base}/findmy/devices/self` request: registers this device, or updates its name and battery. Location is
/// not sent: the cloud does not store it yet.
pub fn report_request(base_url: &str, session_token: &str, name: Option<&str>, battery_percent: Option<u8>, is_charging: Option<bool>) -> Result<URLRequest, CloudError> {
    let mut fields = Vec::new();
    if let Some(n) = name {
        fields.push(("name".to_string(), Json::Str(n.to_string())));
    }
    fields.push(("battery_percent".to_string(), battery_percent.map_or(Json::Null, |p| Json::Num(p.min(100) as f64))));
    fields.push(("is_charging".to_string(), is_charging.map_or(Json::Null, Json::Bool)));
    let mut req = URLRequest::new(&endpoint(base_url, "findmy/devices/self")?);
    req.method = "PUT".into();
    req.body = Some(Json::Obj(fields).encode().into_bytes());
    req.set_header("Content-Type", "application/json");
    with_session(req, session_token)
}

/// Reads the answer to a report: `201` (registered now) or `200` (updated) carries the device, anything else is an error.
pub fn parse_report(status: u16, body: &str) -> Result<(), CloudError> {
    if status == 200 || status == 201 {
        json::parse(body).map(|_| ()).map_err(CloudError::MalformedAnswer)
    } else {
        Err(failure(status, body))
    }
}

/// A command waiting for this device, as the device poll returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCommand {
    pub id: String,
    /// `locate` or `play_sound`. The cloud sends nothing else to a device.
    pub kind: String,
}

/// `GET {base}/findmy/devices/self/commands?wait=N`: the device's pending commands. The service holds the request up
/// to `wait` seconds (at most 25) when nothing is pending.
pub fn commands_request(base_url: &str, session_token: &str, wait_secs: u32) -> Result<URLRequest, CloudError> {
    with_session(URLRequest::new(&endpoint(base_url, &format!("findmy/devices/self/commands?wait={}", wait_secs.min(25)))?), session_token)
}

/// Reads the device poll: `200` with `{"commands": [{"id", "kind", ...}]}`. A command without an id is malformed.
pub fn parse_commands(status: u16, body: &str) -> Result<Vec<PendingCommand>, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let items = parsed.get("commands").and_then(Json::as_array).ok_or_else(|| malformed("no commands"))?;
    items
        .iter()
        .map(|c| -> Result<PendingCommand, CloudError> {
            let id = c.str_field("id").filter(|s| !s.is_empty()).ok_or_else(|| malformed("command has no id"))?;
            Ok(PendingCommand { id, kind: c.str_field("kind").unwrap_or_default() })
        })
        .collect()
}

/// `POST {base}/findmy/commands/{id}/ack`: this device's result for a command, `done` or `failed`.
pub fn ack_request(base_url: &str, session_token: &str, command_id: &str, ok: bool) -> Result<URLRequest, CloudError> {
    let id = checked_id(command_id)?;
    let body = Json::Obj(vec![("status".into(), Json::Str(if ok { "done" } else { "failed" }.into()))]);
    let mut req = URLRequest::post(&endpoint(base_url, &format!("findmy/commands/{id}/ack"))?, body.encode().into_bytes());
    req.set_header("Content-Type", "application/json");
    with_session(req, session_token)
}

/// One entry of the account's event feed: `command`, `command_done`, `share`, `share_received` or `share_ended`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedEvent {
    pub seq: i64,
    pub kind: String,
    pub device_id: Option<String>,
}

/// A page of the feed and the sequence number to ask from next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedPage {
    pub events: Vec<FeedEvent>,
    pub next: i64,
}

/// `GET {base}/findmy/events?after=N&wait=N`: the account's events after `after`, held up to `wait` seconds.
pub fn events_request(base_url: &str, session_token: &str, after: i64, wait_secs: u32) -> Result<URLRequest, CloudError> {
    let path = format!("findmy/events?after={}&wait={}", after.max(0), wait_secs.min(25));
    with_session(URLRequest::new(&endpoint(base_url, &path)?), session_token)
}

/// Reads a feed page: `200` with `{"events": [...], "next": N}`.
pub fn parse_events(status: u16, body: &str) -> Result<FeedPage, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let items = parsed.get("events").and_then(Json::as_array).ok_or_else(|| malformed("no events"))?;
    let events = items
        .iter()
        .map(|e| -> Result<FeedEvent, CloudError> {
            let seq = whole(e, "seq").ok_or_else(|| malformed("event has no seq"))?;
            Ok(FeedEvent { seq, kind: e.str_field("kind").unwrap_or_default(), device_id: e.str_field("device_id") })
        })
        .collect::<Result<Vec<_>, CloudError>>()?;
    let next = whole(&parsed, "next").unwrap_or(0);
    Ok(FeedPage { events, next })
}

/// A sealed position the cloud holds: `sealed` and `enc` are base64 HPKE blobs the cloud cannot open, and `updated` is
/// when the device sent it (Unix seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedBlob {
    pub sealed: String,
    pub enc: String,
    pub updated: i64,
}

/// A position shared with this account by another person, under the share `share_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedLocation {
    pub share_id: String,
    pub owner_name: String,
    pub blob: SealedBlob,
}

/// A device waiting for approval on the account: its own public key, sent so an approving device can seal the
/// account key to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDevice {
    pub id: String,
    pub name: String,
    pub device_public_key: String,
}

/// A JSON request with the session bearer token. `body` is sent as `application/json` when given.
fn json_request(method: &str, url: &str, session_token: &str, body: Option<Json>) -> Result<URLRequest, CloudError> {
    let mut req = URLRequest::new(url);
    req.method = method.into();
    if let Some(b) = body {
        req.body = Some(b.encode().into_bytes());
        req.set_header("Content-Type", "application/json");
    }
    with_session(req, session_token)
}

/// Reads an answer that must have one of the `ok` statuses and carries a non-empty string under `key`.
fn answer_str(status: u16, body: &str, ok: &[u16], key: &str) -> Result<String, CloudError> {
    ok_status(status, body, ok)?;
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    parsed.str_field(key).filter(|s| !s.is_empty()).ok_or_else(|| malformed(&format!("no {key}")))
}

/// Refuses an answer whose status is not one of `ok`, and an answer too large to read.
fn ok_status(status: u16, body: &str, ok: &[u16]) -> Result<(), CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if ok.contains(&status) { Ok(()) } else { Err(failure(status, body)) }
}

fn sealed_blob(j: &Json, sealed_key: &str, enc_key: &str, updated_key: &str) -> Option<SealedBlob> {
    let sealed = j.str_field(sealed_key).filter(|s| !s.is_empty())?;
    let enc = j.str_field(enc_key).filter(|s| !s.is_empty())?;
    Some(SealedBlob { sealed, enc, updated: whole(j, updated_key).unwrap_or(0) })
}

/// `PUT {base}/findmy/keys`: publishes the account's public key (base64 of its 32 raw bytes). Only the public key
/// goes to the cloud; the private key stays in the keychain of the devices that hold it.
pub fn put_account_key_request(base_url: &str, session_token: &str, public_key: &str) -> Result<URLRequest, CloudError> {
    let body = Json::Obj(vec![("public_key".into(), Json::Str(public_key.into()))]);
    json_request("PUT", &endpoint(base_url, "findmy/keys")?, session_token, Some(body))
}

/// `GET {base}/findmy/keys/{account}`: an account's public key, which a contact seals a share to.
pub fn get_account_key_request(base_url: &str, session_token: &str, account: &str) -> Result<URLRequest, CloudError> {
    let id = checked_id(account)?;
    json_request("GET", &endpoint(base_url, &format!("findmy/keys/{id}"))?, session_token, None)
}

/// Reads an account key answer: `200` with `{"public_key": ...}`.
pub fn parse_account_key(status: u16, body: &str) -> Result<String, CloudError> {
    answer_str(status, body, &[200], "public_key")
}

/// `POST {base}/findmy/devices/pending`: asks to join the account with this device's public key. Nothing is approved
/// by the service; an approving device has to seal the account key to it.
pub fn create_pending_request(base_url: &str, session_token: &str, device_public_key: &str, name: &str) -> Result<URLRequest, CloudError> {
    let body = Json::Obj(vec![
        ("device_public_key".into(), Json::Str(device_public_key.into())),
        ("name".into(), Json::Str(name.into())),
    ]);
    json_request("POST", &endpoint(base_url, "findmy/devices/pending")?, session_token, Some(body))
}

/// Reads the id of a new pending device: `200` or `201` with `{"id": ...}`.
pub fn parse_pending_id(status: u16, body: &str) -> Result<String, CloudError> {
    answer_str(status, body, &[200, 201], "id")
}

/// `GET {base}/findmy/devices/pending`: the devices waiting for approval on this account.
pub fn pending_list_request(base_url: &str, session_token: &str) -> Result<URLRequest, CloudError> {
    json_request("GET", &endpoint(base_url, "findmy/devices/pending")?, session_token, None)
}

/// Reads the pending list: `200` with `{"pending": [{"id", "name", "device_public_key"}]}`.
pub fn parse_pending_list(status: u16, body: &str) -> Result<Vec<PendingDevice>, CloudError> {
    ok_status(status, body, &[200])?;
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let items = parsed.get("pending").and_then(Json::as_array).ok_or_else(|| malformed("no pending list"))?;
    items
        .iter()
        .map(|p| -> Result<PendingDevice, CloudError> {
            let id = p.str_field("id").filter(|s| !s.is_empty()).ok_or_else(|| malformed("pending device has no id"))?;
            let device_public_key = p.str_field("device_public_key").filter(|s| !s.is_empty()).ok_or_else(|| malformed("pending device has no key"))?;
            Ok(PendingDevice { id, name: p.str_field("name").unwrap_or_default(), device_public_key })
        })
        .collect()
}

/// `POST {base}/findmy/devices/pending/{id}/key`: stores the account key sealed to the pending device.
pub fn approve_pending_request(base_url: &str, session_token: &str, pending_id: &str, sealed_key: &str) -> Result<URLRequest, CloudError> {
    let id = checked_id(pending_id)?;
    let body = Json::Obj(vec![("sealed_key".into(), Json::Str(sealed_key.into()))]);
    json_request("POST", &endpoint(base_url, &format!("findmy/devices/pending/{id}/key"))?, session_token, Some(body))
}

/// Reads an approve answer: `200`, `201` or `204`.
pub fn parse_approve_pending(status: u16, body: &str) -> Result<(), CloudError> {
    ok_status(status, body, &[200, 201, 204])
}

/// `GET {base}/findmy/devices/pending/{id}/key`: the sealed account key, handed over once. The pending device polls
/// this route until it has an answer.
pub fn sealed_key_request(base_url: &str, session_token: &str, pending_id: &str) -> Result<URLRequest, CloudError> {
    let id = checked_id(pending_id)?;
    json_request("GET", &endpoint(base_url, &format!("findmy/devices/pending/{id}/key"))?, session_token, None)
}

/// Reads the sealed-key poll: `200` with `{"sealed_key": ...}` once approved, `202` while the approval is still
/// waiting (`None`). Any other status, including `404` for a request that was denied or deleted, is an error.
pub fn parse_sealed_key(status: u16, body: &str) -> Result<Option<String>, CloudError> {
    if status == 202 {
        return Ok(None);
    }
    answer_str(status, body, &[200], "sealed_key").map(Some)
}

/// `DELETE {base}/findmy/devices/pending/{id}`: denies a pending device, or clears it after its key was taken.
pub fn delete_pending_request(base_url: &str, session_token: &str, pending_id: &str) -> Result<URLRequest, CloudError> {
    let id = checked_id(pending_id)?;
    json_request("DELETE", &endpoint(base_url, &format!("findmy/devices/pending/{id}"))?, session_token, None)
}

/// Reads a delete answer: `200` or `204`.
pub fn parse_delete(status: u16, body: &str) -> Result<(), CloudError> {
    ok_status(status, body, &[200, 204])
}

/// `PUT {base}/findmy/devices/self/location`: stores this device's sealed position, with its `enc` header.
pub fn put_self_location_request(base_url: &str, session_token: &str, sealed: &str, enc: &str) -> Result<URLRequest, CloudError> {
    let body = Json::Obj(vec![("sealed".into(), Json::Str(sealed.into())), ("enc".into(), Json::Str(enc.into()))]);
    json_request("PUT", &endpoint(base_url, "findmy/devices/self/location")?, session_token, Some(body))
}

/// `DELETE {base}/findmy/devices/self/location`: turning Share My Location off removes the stored position.
pub fn delete_self_location_request(base_url: &str, session_token: &str) -> Result<URLRequest, CloudError> {
    json_request("DELETE", &endpoint(base_url, "findmy/devices/self/location")?, session_token, None)
}

/// `PUT {base}/findmy/shares/{id}/location`: stores the report sealed to one contact, for the share `share_id`.
pub fn put_share_location_request(base_url: &str, session_token: &str, share_id: &str, sealed: &str, enc: &str) -> Result<URLRequest, CloudError> {
    let id = checked_id(share_id)?;
    let body = Json::Obj(vec![("sealed".into(), Json::Str(sealed.into())), ("enc".into(), Json::Str(enc.into()))]);
    json_request("PUT", &endpoint(base_url, &format!("findmy/shares/{id}/location"))?, session_token, Some(body))
}

/// `GET {base}/findmy/shared-with-me`: the positions other people share with this account.
pub fn shared_with_me_request(base_url: &str, session_token: &str) -> Result<URLRequest, CloudError> {
    json_request("GET", &endpoint(base_url, "findmy/shared-with-me")?, session_token, None)
}

/// Reads the shared list: `200` with a JSON array of `{"share_id", "owner_name", "sealed", "enc", "updated"}`. An entry
/// without its share id or its blob is malformed.
pub fn parse_shared_with_me(status: u16, body: &str) -> Result<Vec<SharedLocation>, CloudError> {
    ok_status(status, body, &[200])?;
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let items = parsed.as_array().ok_or_else(|| malformed("the shared list is not an array"))?;
    items
        .iter()
        .map(|s| -> Result<SharedLocation, CloudError> {
            let share_id = s.str_field("share_id").filter(|v| !v.is_empty()).ok_or_else(|| malformed("shared entry has no share_id"))?;
            let blob = sealed_blob(s, "sealed", "enc", "updated").ok_or_else(|| malformed("shared entry has no sealed location"))?;
            Ok(SharedLocation { share_id, owner_name: s.str_field("owner_name").unwrap_or_default(), blob })
        })
        .collect()
}

#[cfg(test)]
mod keys_tests {
    use super::*;

    const BASE: &str = "https://bifrost.olibuijr.com/api/cloud/v1";
    const TOKEN: &str = "tok42";

    fn body_of(req: &URLRequest) -> String {
        String::from_utf8(req.body.clone().unwrap_or_default()).unwrap()
    }

    #[test]
    fn the_account_key_is_put_as_a_public_key_only() {
        let req = put_account_key_request(BASE, TOKEN, "cHVi").unwrap();
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, format!("{BASE}/findmy/keys"));
        assert_eq!(body_of(&req), r#"{"public_key":"cHVi"}"#);
        assert_eq!(get_account_key_request(BASE, TOKEN, "a1").unwrap().url, format!("{BASE}/findmy/keys/a1"));
        assert!(get_account_key_request(BASE, TOKEN, "../x").is_err());
        assert_eq!(parse_account_key(200, r#"{"public_key":"cHVi"}"#).unwrap(), "cHVi");
        assert!(parse_account_key(404, r#"{"error":"no_account"}"#).is_err());
    }

    #[test]
    fn a_pending_device_posts_its_key_and_name_and_reads_back_an_id() {
        let req = create_pending_request(BASE, TOKEN, "a2V5", "Midget").unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, format!("{BASE}/findmy/devices/pending"));
        assert_eq!(body_of(&req), r#"{"device_public_key":"a2V5","name":"Midget"}"#);
        assert_eq!(parse_pending_id(201, r#"{"id":"p7"}"#).unwrap(), "p7");
        assert!(parse_pending_id(201, r#"{}"#).is_err());
    }

    #[test]
    fn the_pending_list_reads_each_device_with_its_key() {
        let req = pending_list_request(BASE, TOKEN).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, format!("{BASE}/findmy/devices/pending"));
        let list = parse_pending_list(200, r#"{"pending":[{"id":"p1","name":"Tab","device_public_key":"a2V5"}]}"#).unwrap();
        assert_eq!(list, vec![PendingDevice { id: "p1".into(), name: "Tab".into(), device_public_key: "a2V5".into() }]);
        assert!(parse_pending_list(200, r#"{"pending":[{"id":"p1"}]}"#).is_err(), "a pending device needs its key");
    }

    #[test]
    fn approval_posts_the_sealed_key_and_the_poll_reads_it_or_waits() {
        let req = approve_pending_request(BASE, TOKEN, "p1", "c2VhbGVk").unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/devices/pending/p1/key"));
        assert_eq!(body_of(&req), r#"{"sealed_key":"c2VhbGVk"}"#);
        assert!(parse_approve_pending(200, "{}").is_ok());
        let poll = sealed_key_request(BASE, TOKEN, "p1").unwrap();
        assert_eq!(poll.method, "GET");
        assert_eq!(parse_sealed_key(200, r#"{"sealed_key":"c2VhbGVk"}"#).unwrap(), Some("c2VhbGVk".into()));
        assert_eq!(parse_sealed_key(202, "{}").unwrap(), None, "still waiting");
        assert!(parse_sealed_key(404, r#"{"error":"gone"}"#).is_err(), "a denied request is an error to the poll");
    }

    #[test]
    fn denying_a_pending_device_deletes_it() {
        let req = delete_pending_request(BASE, TOKEN, "p1").unwrap();
        assert_eq!(req.method, "DELETE");
        assert_eq!(req.url, format!("{BASE}/findmy/devices/pending/p1"));
        assert!(parse_delete(204, "").is_ok());
        assert!(parse_delete(404, r#"{"error":"gone"}"#).is_err());
    }

    #[test]
    fn the_self_location_is_put_sealed_and_deleted_when_sharing_stops() {
        let req = put_self_location_request(BASE, TOKEN, "s1", "e1").unwrap();
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, format!("{BASE}/findmy/devices/self/location"));
        assert_eq!(body_of(&req), r#"{"sealed":"s1","enc":"e1"}"#);
        let del = delete_self_location_request(BASE, TOKEN).unwrap();
        assert_eq!(del.method, "DELETE");
        assert_eq!(del.url, format!("{BASE}/findmy/devices/self/location"));
        assert_eq!(del.headers.iter().find(|(k, _)| k.as_str() == "Authorization").map(|(_, v)| v.as_str()), Some("Bearer tok42"));
    }

    #[test]
    fn a_contact_copy_goes_to_the_share_route() {
        let req = put_share_location_request(BASE, TOKEN, "sh9", "s2", "e2").unwrap();
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, format!("{BASE}/findmy/shares/sh9/location"));
        assert_eq!(body_of(&req), r#"{"sealed":"s2","enc":"e2"}"#);
        assert!(put_share_location_request(BASE, TOKEN, "../sh", "s", "e").is_err());
    }

    #[test]
    fn shared_with_me_reads_an_array_of_sealed_positions() {
        let req = shared_with_me_request(BASE, TOKEN).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/shared-with-me"));
        let body = r#"[{"share_id":"sh9","owner_name":"Anna","sealed":"s2","enc":"e2","updated":1700000000}]"#;
        let list = parse_shared_with_me(200, body).unwrap();
        assert_eq!(
            list,
            vec![SharedLocation {
                share_id: "sh9".into(),
                owner_name: "Anna".into(),
                blob: SealedBlob { sealed: "s2".into(), enc: "e2".into(), updated: 1_700_000_000 },
            }]
        );
        assert!(parse_shared_with_me(200, r#"[{"share_id":"sh9","owner_name":"Anna"}]"#).is_err(), "no blob is malformed");
        assert!(parse_shared_with_me(200, r#"{"shared":[]}"#).is_err(), "the answer is an array");
    }

    #[test]
    fn the_device_list_carries_the_sealed_location_when_present() {
        let body = r#"{"devices":[{"id":"d1","location_sealed":"s1","location_enc":"e1","location_updated":1700000500},
            {"id":"d2","location_sealed":null,"location_enc":null}]}"#;
        let list = parse_devices(200, body).unwrap();
        assert_eq!(list.devices[0].sealed_location, Some(SealedBlob { sealed: "s1".into(), enc: "e1".into(), updated: 1_700_000_500 }));
        assert_eq!(list.devices[1].sealed_location, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://bifrost.olibuijr.com/api/cloud/v1";
    const TOKEN: &str = "abc123";

    fn header<'a>(req: &'a URLRequest, name: &str) -> Option<&'a str> {
        req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    #[test]
    fn devices_request_sends_the_bearer_token_to_the_list_route() {
        let req = devices_request(BASE, TOKEN).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/devices"));
        assert_eq!(req.method, "GET");
        assert_eq!(header(&req, "Authorization"), Some("Bearer abc123"));
        assert_eq!(header(&req, "Cookie"), None);
    }

    #[test]
    fn a_session_token_with_symbols_is_refused_before_sending() {
        assert!(devices_request(BASE, "abc; path=/").is_err());
        assert!(devices_request(BASE, "").is_err());
    }

    #[test]
    fn a_plain_http_base_is_refused() {
        assert!(devices_request("http://example.com/api/cloud/v1", TOKEN).is_err());
    }

    #[test]
    fn parse_devices_reads_each_device_with_its_battery_and_fix() {
        let body = r#"{"this_device":"d1","devices":[
            {"id":"d1","name":"Midget","model":"M900","last_seen":1700000000,"battery_percent":82,"is_charging":true,
             "location":{"latitude":64.14,"longitude":-21.94,"accuracy_m":12,"recorded_at":1700000100}},
            {"id":"d2","name":"Tab","model":"A8","last_seen":1600000000,"battery_percent":null,"is_charging":null,"location":null}
        ]}"#;
        let list = parse_devices(200, body).unwrap();
        assert_eq!(list.this_device.as_deref(), Some("d1"));
        assert_eq!(list.devices.len(), 2);
        let first = &list.devices[0];
        assert_eq!(first.name, "Midget");
        assert_eq!(first.battery_percent, Some(82));
        assert_eq!(first.is_charging, Some(true));
        assert_eq!(first.fix, Some(CloudFix { latitude: 64.14, longitude: -21.94, accuracy_m: Some(12.0), recorded_at: 1_700_000_100 }));
        assert_eq!(list.devices[1].fix, None);
        assert_eq!(list.devices[1].battery_percent, None);
    }

    #[test]
    fn parse_devices_refuses_bad_coordinates_and_missing_ids() {
        let bad_lat = r#"{"devices":[{"id":"d1","location":{"latitude":95,"longitude":0}}]}"#;
        assert!(matches!(parse_devices(200, bad_lat), Err(CloudError::MalformedAnswer(_))));
        let no_id = r#"{"devices":[{"name":"x"}]}"#;
        assert!(matches!(parse_devices(200, no_id), Err(CloudError::MalformedAnswer(_))));
    }

    #[test]
    fn parse_devices_maps_refusals_to_their_errors() {
        assert_eq!(parse_devices(401, r#"{"error":"unauthorized"}"#), Err(CloudError::Rejected { status: 401, code: "unauthorized".into() }));
        assert!(parse_devices(200, "not json").is_err());
    }

    #[test]
    fn command_request_queues_locate_and_play_sound_without_confirmation() {
        let req = command_request(BASE, TOKEN, "d1", CommandKind::PlaySound, false).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/devices/d1/commands"));
        assert_eq!(req.method, "POST");
        let body = String::from_utf8(req.body.unwrap()).unwrap();
        assert!(body.contains("\"kind\":\"play_sound\""), "{body}");
        assert!(body.contains("\"confirmed\":false"), "{body}");
        assert!(command_request(BASE, TOKEN, "d1", CommandKind::Locate, false).is_ok());
    }

    #[test]
    fn erase_needs_confirmation_and_a_clean_device_id() {
        assert!(command_request(BASE, TOKEN, "d1", CommandKind::Erase, false).is_err());
        let req = command_request(BASE, TOKEN, "d1", CommandKind::Erase, true).unwrap();
        assert!(String::from_utf8(req.body.unwrap()).unwrap().contains("\"kind\":\"erase\""));
        assert!(command_request(BASE, TOKEN, "../etc", CommandKind::Locate, false).is_err());
    }

    #[test]
    fn parse_command_returns_the_queued_id() {
        assert_eq!(parse_command(201, r#"{"command":{"id":"c9"}}"#).unwrap(), "c9");
        assert!(parse_command(201, r#"{"command":{}}"#).is_err());
        assert_eq!(parse_command(404, r#"{"error":"no_device"}"#), Err(CloudError::Rejected { status: 404, code: "no_device".into() }));
    }

    #[test]
    fn report_request_puts_name_and_battery_to_the_device_route_and_no_location() {
        let req = report_request(BASE, TOKEN, Some("Midget"), Some(70), Some(false)).unwrap();
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, format!("{BASE}/findmy/devices/self"));
        let body = String::from_utf8(req.body.unwrap()).unwrap();
        assert!(body.contains("\"name\":\"Midget\""), "{body}");
        assert!(body.contains("\"battery_percent\":70"), "{body}");
        assert!(!body.contains("location"), "no position is sent: {body}");
        let bare = report_request(BASE, TOKEN, None, None, None).unwrap();
        assert!(!String::from_utf8(bare.body.unwrap()).unwrap().contains("name"));
    }

    #[test]
    fn parse_report_accepts_registered_and_updated_devices() {
        assert!(parse_report(201, r#"{"device":{"id":"d1"}}"#).is_ok());
        assert!(parse_report(200, r#"{"device":{"id":"d1"}}"#).is_ok());
        assert!(parse_report(403, r#"{"error":"not_a_device"}"#).is_err());
    }

    #[test]
    fn the_device_poll_asks_with_a_wait_and_reads_the_commands() {
        let req = commands_request(BASE, TOKEN, 99).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/devices/self/commands?wait=25"), "the wait is capped at 25 s");
        assert_eq!(req.method, "GET");
        let cmds = parse_commands(200, r#"{"commands":[{"id":"c1","kind":"play_sound"},{"id":"c2","kind":"locate"}]}"#).unwrap();
        assert_eq!(cmds, vec![PendingCommand { id: "c1".into(), kind: "play_sound".into() }, PendingCommand { id: "c2".into(), kind: "locate".into() }]);
        assert!(parse_commands(200, r#"{"commands":[]}"#).unwrap().is_empty());
        assert!(parse_commands(200, r#"{"commands":[{"kind":"locate"}]}"#).is_err(), "a command needs an id");
        assert!(parse_commands(403, r#"{"error":"not_a_device"}"#).is_err());
    }

    #[test]
    fn an_ack_says_done_or_failed_for_a_clean_command_id() {
        let req = ack_request(BASE, TOKEN, "c1", true).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/commands/c1/ack"));
        assert!(String::from_utf8(req.body.unwrap()).unwrap().contains("\"status\":\"done\""));
        let failed = ack_request(BASE, TOKEN, "c1", false).unwrap();
        assert!(String::from_utf8(failed.body.unwrap()).unwrap().contains("\"status\":\"failed\""));
        assert!(ack_request(BASE, TOKEN, "../x", true).is_err());
    }

    #[test]
    fn the_event_feed_is_asked_from_a_sequence_number_and_read_back() {
        let req = events_request(BASE, TOKEN, 7, 0).unwrap();
        assert_eq!(req.url, format!("{BASE}/findmy/events?after=7&wait=0"));
        let page = parse_events(200, r#"{"events":[{"seq":8,"kind":"command_done","device_id":"d1"},{"seq":9,"kind":"share_received","device_id":null}],"next":9}"#).unwrap();
        assert_eq!(page.next, 9);
        assert_eq!(page.events[0], FeedEvent { seq: 8, kind: "command_done".into(), device_id: Some("d1".into()) });
        assert_eq!(page.events[1].device_id, None);
        assert!(parse_events(200, r#"{"events":[{"kind":"x"}],"next":0}"#).is_err(), "an event needs a seq");
    }
}
