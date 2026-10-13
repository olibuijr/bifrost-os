//! The HTTP side of the Matrix client: one [`Api`] per signed-in account. Calls go through the shell's HTTPS client
//! (`springboard::deildu::http`), so certificates are checked against the same roots as the rest of the desktop. The
//! URLs and bodies come from [`crate::protocol`]. Every method blocks, so the window calls them from job threads
//! (see [`crate::sync`]).

use crate::protocol::{self, Login, Sync};
use bifrost_core::json::{self, Json};
use springboard::deildu::http::{Client, Jar, Response, Transport};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The largest sync answer read. A first sync of a busy account is large.
const SYNC_LIMIT: usize = 32 * 1024 * 1024;
/// The largest answer of any other call.
const CALL_LIMIT: usize = 1024 * 1024;
/// The largest image the app uploads.
pub const UPLOAD_LIMIT: usize = 25 * 1024 * 1024;

/// A signed-in session on one homeserver.
pub struct Api {
    http: Client,
    base: String,
    token: String,
    pub user_id: String,
    pub device_id: String,
}

impl Api {
    /// Signs in with a user name and password. Makes a new device on the account, so the token is per start.
    pub fn login(transport: Arc<dyn Transport>, base: &str, user: &str, password: &str) -> Result<Api, String> {
        let http = Client::new(transport, Jar::default()).require_https();
        let body = protocol::login_body(user, password);
        let url = protocol::login_url(base);
        let answer = call(&http, "POST", &url, None, Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        let Login { user_id, access_token, device_id } = protocol::parse_login(&answer)?;
        Ok(Api { http, base: base.to_string(), token: access_token, user_id, device_id })
    }

    /// One sync: the changes after `since`, or the whole state when there is no token. Long-polls for up to
    /// [`protocol::SYNC_TIMEOUT_MS`].
    pub fn sync(&self, since: Option<&str>) -> Result<Sync, String> {
        let url = protocol::sync_url(&self.base, since);
        let answer = call(&self.http, "GET", &url, Some(self.token.as_str()), None, SYNC_LIMIT)?;
        protocol::parse_sync(&answer)
    }

    /// Sends a text message to `room`.
    pub fn send_text(&self, room: &str, text: &str) -> Result<(), String> {
        self.send_content(room, &protocol::text_content(text))
    }

    /// Uploads an image and sends it to `room` as an image message.
    pub fn send_image(&self, room: &str, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        let mxc = self.upload(name, mime, bytes)?;
        let info = protocol::ImageInfo { name, mxc: &mxc, mime, size: bytes.len() as u64 };
        self.send_content(room, &protocol::image_content(&info))
    }

    /// Uploads any file and sends it to `room` as a file message.
    pub fn send_file(&self, room: &str, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        let mxc = self.upload(name, mime, bytes)?;
        let info = protocol::ImageInfo { name, mxc: &mxc, mime, size: bytes.len() as u64 };
        self.send_content(room, &protocol::file_content(&info))
    }

    /// Sends a text message that answers the message `reply_to`.
    pub fn send_reply(&self, room: &str, text: &str, reply_to: &str) -> Result<(), String> {
        self.send_content(room, &protocol::reply_content(text, reply_to))
    }

    /// Replaces the words of the message `target` with `text`.
    pub fn send_edit(&self, room: &str, target: &str, text: &str) -> Result<(), String> {
        self.send_content(room, &protocol::edit_content(target, text))
    }

    /// Reacts to the message `target` with `key` (an emoji).
    pub fn send_reaction(&self, room: &str, target: &str, key: &str) -> Result<(), String> {
        let url = protocol::send_event_url(&self.base, room, "m.reaction", &txn_id());
        let content = protocol::reaction_content(target, key);
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((content.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Removes (redacts) `event`: Undo Send for a message, or taking back a reaction.
    pub fn redact(&self, room: &str, event: &str) -> Result<(), String> {
        let url = protocol::redact_url(&self.base, room, event, &txn_id());
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some(("{}".as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Makes a private conversation with `user_id` and returns its room id.
    pub fn create_direct(&self, user_id: &str) -> Result<String, String> {
        let url = protocol::create_room_url(&self.base);
        let body = protocol::create_direct_body(user_id);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_room_id(&answer)
    }

    /// Leaves `room`, then forgets it so it does not come back in the list. A failed forget is not an error: the room
    /// is left either way.
    pub fn leave(&self, room: &str) -> Result<(), String> {
        let empty = Some(("{}".as_bytes(), "application/json"));
        call(&self.http, "POST", &protocol::leave_url(&self.base, room), Some(self.token.as_str()), empty, CALL_LIMIT)?;
        let _ = call(&self.http, "POST", &protocol::forget_url(&self.base, room), Some(self.token.as_str()), empty, CALL_LIMIT);
        Ok(())
    }

    /// Renames `room`.
    pub fn rename_room(&self, room: &str, name: &str) -> Result<(), String> {
        let url = protocol::room_name_url(&self.base, room);
        let body = protocol::room_name_body(name);
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Ends this device's session on the homeserver. The access token stops working.
    pub fn logout(&self) -> Result<(), String> {
        let empty = Some(("{}".as_bytes(), "application/json"));
        call(&self.http, "POST", &protocol::logout_url(&self.base), Some(self.token.as_str()), empty, CALL_LIMIT).map(|_| ())
    }

    /// Marks `event` as read in `room` (a public read receipt).
    pub fn read_receipt(&self, room: &str, event: &str) -> Result<(), String> {
        let url = protocol::receipt_url(&self.base, room, event);
        call(&self.http, "POST", &url, Some(self.token.as_str()), Some(("{}".as_bytes(), "application/json")), CALL_LIMIT)
            .map(|_| ())
    }

    fn send_content(&self, room: &str, content: &str) -> Result<(), String> {
        let url = protocol::send_url(&self.base, room, &txn_id());
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((content.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    fn upload(&self, name: &str, mime: &str, bytes: &[u8]) -> Result<String, String> {
        let url = protocol::upload_url(&self.base, name);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((bytes, mime)), CALL_LIMIT)?;
        protocol::parse_content_uri(&answer)
    }
}

/// One API call. `token` is sent as a bearer token; `body` is the bytes and their content type. A 2xx answer is parsed
/// as JSON; any other answer becomes its error text, or the status when it has none.
fn call(
    http: &Client,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<(&[u8], &str)>,
    limit: usize,
) -> Result<Json, String> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(a) = &auth {
        headers.push(("Authorization", a.as_str()));
    }
    if let Some((_, content_type)) = body {
        headers.push(("Content-Type", content_type));
    }
    let bytes = body.map(|(b, _)| b);
    let response = http.request_with_headers(method, url, &headers, bytes, limit).map_err(|e| e.to_string())?;
    answer(&response)
}

fn answer(response: &Response) -> Result<Json, String> {
    let text = String::from_utf8_lossy(&response.body);
    let parsed = json::parse(&text);
    if (200..300).contains(&response.status) {
        return parsed.map_err(|_| format!("the server answered with text that is not JSON (HTTP {})", response.status));
    }
    match parsed {
        Ok(body) => Err(protocol::error_text(&body).unwrap_or_else(|| format!("the server answered HTTP {}", response.status))),
        Err(_) => Err(format!("the server answered HTTP {}", response.status)),
    }
}

/// A transaction id unique to this run: the time and a counter.
fn txn_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("bifrost{ms}-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> Response {
        Response { status, body: body.as_bytes().to_vec(), ..Default::default() }
    }

    #[test]
    fn a_success_answer_is_its_json() {
        let json = answer(&response(200, r#"{"event_id":"$x"}"#)).unwrap();
        assert_eq!(json.str_field("event_id").as_deref(), Some("$x"));
    }

    #[test]
    fn an_error_answer_says_what_the_server_said() {
        let err = answer(&response(403, r#"{"errcode":"M_FORBIDDEN","error":"no"}"#)).unwrap_err();
        assert_eq!(err, "M_FORBIDDEN: no");
        assert_eq!(answer(&response(502, "<html>bad gateway</html>")).unwrap_err(), "the server answered HTTP 502");
    }

    #[test]
    fn a_success_that_is_not_json_is_an_error() {
        assert!(answer(&response(200, "<html>")).unwrap_err().contains("not JSON"));
    }

    #[test]
    fn transaction_ids_do_not_repeat() {
        assert_ne!(txn_id(), txn_id());
    }
}
