//! Universal Clipboard against the BifrOSt Cloud HTTP API: the request a device sends to put its sealed copy on the
//! account (`PUT {base}/clipboard`), the long-poll that reads the newest copy after a sequence number (`GET
//! {base}/clipboard?after=N&wait=S`), and the answers those read back. The cloud keeps one item per account and deletes
//! it after two minutes; it cannot open what it holds. The sealing is `bifrost_app::cloud_keys` (AAD
//! `bifrost.clipboard.v1`), and the policy (what may be shared, when an item is offered) is `springboard::universal_clipboard`.
//!
//! Everything here is pure: requests are built and answers read with no socket. Each call blocks; run it on a background
//! thread. The service side lives in bifrost-cloud (`clipboard` routes).

use crate::cloud_account::{checked_token, endpoint, failure, CloudError};
use crate::url::URLRequest;
use bifrost_core::json::{self, Json};

/// The largest sealed copy that may be sent, in bytes of base64 (a 4 MiB image seals to about this much).
pub const MAX_BLOB_BYTES: usize = 8 * 1024 * 1024;
/// The largest answer read: one item, which can hold a whole sealed image.
pub const MAX_ANSWER_BYTES: usize = MAX_BLOB_BYTES + 64 * 1024;
/// The longest a long-poll waits, in seconds.
pub const MAX_WAIT_SECS: u32 = 25;

/// The copy the cloud holds for the account: its sequence number, the device that sent it, the sealed blob (`sealed` and
/// `enc`, base64), and when the cloud deletes it (Unix seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardItem {
    pub seq: i64,
    pub device_id: String,
    pub sealed: String,
    pub enc: String,
    pub expires_at: i64,
}

/// The cloud's receipt for a put: the sequence number it gave the copy and when it will be deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub seq: i64,
    pub expires_at: i64,
}

fn malformed(what: &str) -> CloudError {
    CloudError::MalformedAnswer(what.to_string())
}

fn with_session(mut req: URLRequest, session_token: &str) -> Result<URLRequest, CloudError> {
    let token = checked_token(session_token)?;
    req.set_header("Authorization", &format!("Bearer {token}"));
    req.set_header("Accept", "application/json");
    Ok(req)
}

/// The `PUT {base}/clipboard` request that replaces the account's copy with `sealed` and `enc` (base64, from
/// `cloud_keys::seal`). Refused before a request is built when the blob is empty or larger than [`MAX_BLOB_BYTES`].
pub fn put_clipboard_request(base_url: &str, session_token: &str, sealed: &str, enc: &str) -> Result<URLRequest, CloudError> {
    if sealed.is_empty() || enc.is_empty() || sealed.len() + enc.len() > MAX_BLOB_BYTES {
        return Err(malformed("the copy is empty or too large"));
    }
    let mut req = URLRequest::new(&endpoint(base_url, "clipboard")?);
    req.method = "PUT".into();
    let body = Json::Obj(vec![("sealed".into(), Json::Str(sealed.into())), ("enc".into(), Json::Str(enc.into()))]);
    req.body = Some(body.encode().into_bytes());
    req.set_header("Content-Type", "application/json");
    with_session(req, session_token)
}

/// Reads the receipt of a put: `200` with `{"seq": N, "expires_at": T}`.
pub fn parse_put_clipboard(status: u16, body: &str) -> Result<Receipt, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let j = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let seq = j.get("seq").and_then(Json::number).ok_or_else(|| malformed("no seq"))? as i64;
    let expires_at = j.get("expires_at").and_then(Json::number).ok_or_else(|| malformed("no expires_at"))? as i64;
    Ok(Receipt { seq, expires_at })
}

/// The long-poll `GET {base}/clipboard?after=N&wait=S`: the account's copy when its sequence number is above `after`,
/// else the answer waits up to `wait_secs` (at most [`MAX_WAIT_SECS`]) for one.
pub fn get_clipboard_request(base_url: &str, session_token: &str, after: i64, wait_secs: u32) -> Result<URLRequest, CloudError> {
    let path = format!("clipboard?after={}&wait={}", after.max(0), wait_secs.min(MAX_WAIT_SECS));
    with_session(URLRequest::new(&endpoint(base_url, &path)?), session_token)
}

/// Reads a long-poll answer: `200` with `{"item": null}` when nothing newer came, or `{"item": {...}}`.
pub fn parse_get_clipboard(status: u16, body: &str) -> Result<Option<ClipboardItem>, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(malformed("the answer is too large"));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let j = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let item = match j.get("item") {
        None => return Err(malformed("no item")),
        Some(Json::Null) => return Ok(None),
        Some(item) => item,
    };
    let text = |k: &str| item.str_field(k).filter(|s| !s.is_empty()).ok_or_else(|| malformed(&format!("no {k}")));
    let num = |k: &str| item.get(k).and_then(Json::number).map(|n| n as i64).ok_or_else(|| malformed(&format!("no {k}")));
    Ok(Some(ClipboardItem {
        seq: num("seq")?,
        device_id: text("device_id")?,
        sealed: text("sealed")?,
        enc: text("enc")?,
        expires_at: num("expires_at")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://bifrost.olibuijr.com/api/cloud/v1";
    const TOKEN: &str = "tok42";

    #[test]
    fn a_copy_is_put_as_a_sealed_blob_under_the_session() {
        let req = put_clipboard_request(BASE, TOKEN, "c2VhbGVk", "ZW5j").unwrap();
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, format!("{BASE}/clipboard"));
        assert_eq!(String::from_utf8(req.body.clone().unwrap()).unwrap(), r#"{"sealed":"c2VhbGVk","enc":"ZW5j"}"#);
    }

    #[test]
    fn an_empty_or_oversize_copy_is_refused_before_a_request_is_built() {
        assert!(put_clipboard_request(BASE, TOKEN, "", "ZW5j").is_err());
        let huge = "A".repeat(MAX_BLOB_BYTES);
        assert!(put_clipboard_request(BASE, TOKEN, &huge, "ZW5j").is_err());
        assert!(put_clipboard_request("http://insecure.example/api", TOKEN, "c2VhbGVk", "ZW5j").is_err());
    }

    #[test]
    fn the_receipt_carries_the_sequence_and_the_expiry() {
        assert_eq!(parse_put_clipboard(200, r#"{"seq":7,"expires_at":1120}"#).unwrap(), Receipt { seq: 7, expires_at: 1120 });
        assert!(parse_put_clipboard(200, r#"{"seq":7}"#).is_err());
        assert!(parse_put_clipboard(401, r#"{"error":"unauthorized"}"#).is_err());
    }

    #[test]
    fn the_long_poll_names_the_sequence_and_caps_the_wait() {
        let req = get_clipboard_request(BASE, TOKEN, 41, 999).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, format!("{BASE}/clipboard?after=41&wait={MAX_WAIT_SECS}"));
        let first = get_clipboard_request(BASE, TOKEN, -5, 0).unwrap();
        assert_eq!(first.url, format!("{BASE}/clipboard?after=0&wait=0"));
    }

    #[test]
    fn an_empty_long_poll_reads_as_no_copy() {
        assert_eq!(parse_get_clipboard(200, r#"{"item":null}"#).unwrap(), None);
    }

    #[test]
    fn a_copy_reads_back_with_its_device_and_expiry() {
        let body = r#"{"item":{"seq":9,"device_id":"d1","sealed":"c2VhbGVk","enc":"ZW5j","expires_at":1120}}"#;
        assert_eq!(
            parse_get_clipboard(200, body).unwrap(),
            Some(ClipboardItem { seq: 9, device_id: "d1".into(), sealed: "c2VhbGVk".into(), enc: "ZW5j".into(), expires_at: 1120 })
        );
        assert!(parse_get_clipboard(200, r#"{"item":{"seq":9}}"#).is_err(), "a copy without its blob is malformed");
        assert!(parse_get_clipboard(200, r#"{}"#).is_err());
    }

    #[test]
    fn a_failed_long_poll_is_an_error_not_an_empty_clipboard() {
        assert!(parse_get_clipboard(503, r#"{"error":"busy"}"#).is_err());
        assert!(parse_get_clipboard(200, &"x".repeat(MAX_ANSWER_BYTES + 1)).is_err());
    }
}
