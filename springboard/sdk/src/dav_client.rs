//! CalDAV and CardDAV client for the apps that sync Internet Accounts (Calendar and Contacts).
//!
//! Each call is blocking: run it on a background thread. It discovers the user's principal, the home set of the kind
//! and its collections (RFC 6764 and RFC 4791 and RFC 6352 names), then reads every event or card. The protocol parts
//! are in `bifrost_ui::internet_accounts::dav`. The transport is a `send` function, so the walk is tested with canned
//! answers; [`session_send`] is the one that uses the URL session.

use crate::controls::internet_accounts::{dav, Kind};
use crate::http_auth::base64_encode;
use crate::url::{HTTPURLResponse, URLRequest, URLSession};
use std::sync::atomic::AtomicBool;

/// Most calendar objects or cards read in one sync.
pub const MAX_ITEMS: usize = 10_000;

/// The `Authorization` header value for a login (HTTP Basic).
pub fn basic_auth(user: &str, password: &str) -> String {
    format!("Basic {}", base64_encode(format!("{user}:{password}").as_bytes()))
}

/// Sends one request and reads its status and body text. The URL session implements it in [`session_send`].
pub type Send<'a> = dyn FnMut(&URLRequest) -> Result<(u16, String), String> + 'a;

/// The URL session transport: blocking, reads the whole answer up to [`dav::MAX_ANSWER_BYTES`].
pub fn session_send(req: &URLRequest) -> Result<(u16, String), String> {
    let cancel = AtomicBool::new(false);
    let mut body: Vec<u8> = Vec::new();
    let head: HTTPURLResponse = URLSession::shared()
        .bytes(
            req,
            &cancel,
            &mut |_: &HTTPURLResponse| {},
            &mut |chunk: &[u8]| {
                body.extend_from_slice(chunk);
                body.len() <= dav::MAX_ANSWER_BYTES
            },
        )
        .map_err(|e| e.to_string())?;
    Ok((head.status_code, String::from_utf8_lossy(&body).into_owned()))
}

fn request(method: &str, url: &str, auth: &str, depth: &str, body: String) -> URLRequest {
    let mut req = URLRequest { method: method.to_string(), body: Some(body.into_bytes()), ..URLRequest::new(url) };
    req.set_header("Authorization", auth);
    req.set_header("Depth", depth);
    req.set_header("Content-Type", "application/xml; charset=utf-8");
    req
}

/// Sends `req` and reads the multistatus answer. Any status other than 2xx is an error.
fn answer(send: &mut Send<'_>, req: &URLRequest) -> Result<Vec<dav::Response>, String> {
    let (status, body) = send(req)?;
    if !(200..300).contains(&status) {
        return Err(format!("The server answered {status} to {}", req.method));
    }
    Ok(dav::parse_multistatus(&body))
}

/// The first href a property names, resolved against the URL it was asked of.
fn first_href(responses: &[dav::Response], name: &str, base: &str) -> Option<String> {
    responses.iter().find_map(|r| r.prop(name)).and_then(|v| dav::hrefs(v).first().map(|h| dav::resolve(base, h)))
}

/// The user's principal URL: asked of `url` itself, then of the well-known path of its server.
fn find_principal(send: &mut Send<'_>, url: &str, auth: &str) -> Result<String, String> {
    let req = request("PROPFIND", url, auth, "0", dav::principal_body());
    let found = answer(send, &req).ok().and_then(|rs| first_href(&rs, "current-user-principal", url));
    found.ok_or_else(|| "The server did not name a user principal".to_string())
}

/// The collections of `kind` under the account's server: calendars for CalDAV, address books for CardDAV.
pub fn collections(send: &mut Send<'_>, base: &str, kind: Kind, auth: &str) -> Result<Vec<String>, String> {
    let principal = match find_principal(send, base, auth) {
        Ok(p) => p,
        Err(first) => match dav::origin(base) {
            Some(origin) => find_principal(send, &format!("{origin}{}", dav::well_known_path(kind)), auth).map_err(|_| first)?,
            None => return Err(first),
        },
    };
    let req = request("PROPFIND", &principal, auth, "0", dav::home_set_body(kind));
    let home_prop = if kind == Kind::CardDav { "addressbook-home-set" } else { "calendar-home-set" };
    let home = first_href(&answer(send, &req)?, home_prop, &principal).ok_or_else(|| "The server did not name a home folder".to_string())?;
    let req = request("PROPFIND", &home, auth, "1", dav::collections_body());
    Ok(answer(send, &req)?.iter().filter(|r| dav::collection_kind(r) == Some(kind)).map(|r| dav::resolve(&home, &r.href)).collect())
}

/// The iCalendar text of every event that overlaps `start..end` (Unix seconds) in every calendar of the account.
pub fn calendar_texts(send: &mut Send<'_>, base: &str, auth: &str, start: i64, end: i64) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for calendar in collections(send, base, Kind::CalDav, auth)? {
        let req = request("REPORT", &calendar, auth, "1", dav::calendar_query_body(start, end));
        for r in answer(send, &req)? {
            if let Some(text) = r.prop("calendar-data").filter(|t| !t.trim().is_empty()) {
                out.push(text.to_string());
                if out.len() >= MAX_ITEMS {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

/// The vCard text of every card in every address book of the account.
pub fn card_texts(send: &mut Send<'_>, base: &str, auth: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for book in collections(send, base, Kind::CardDav, auth)? {
        let req = request("REPORT", &book, auth, "1", dav::addressbook_query_body());
        for r in answer(send, &req)? {
            if let Some(text) = r.prop("address-data").filter(|t| !t.trim().is_empty()) {
                out.push(text.to_string());
                if out.len() >= MAX_ITEMS {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://dav.example.com/";
    const PRINCIPAL: &str = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop>
        <d:current-user-principal><d:href>/principals/olibuijr/</d:href></d:current-user-principal></d:prop>
        <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
    const HOME: &str = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response>
        <d:href>/principals/olibuijr/</d:href><d:propstat><d:prop><c:calendar-home-set><d:href>/calendars/olibuijr/</d:href>
        </c:calendar-home-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
    const COLLECTIONS: &str = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:card="urn:ietf:params:xml:ns:carddav">
        <d:response><d:href>/calendars/olibuijr/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>
        </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        <d:response><d:href>/calendars/olibuijr/home/</d:href><d:propstat><d:prop><d:displayname>Home</d:displayname>
        <d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        <d:response><d:href>/calendars/olibuijr/contacts/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/>
        <card:addressbook/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
    const EVENTS: &str = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response>
        <d:href>/calendars/olibuijr/home/a.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
        END:VCALENDAR</c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

    /// A server that answers the routes above and refuses anything else.
    fn server(method: &str, url: &str) -> Result<(u16, String), String> {
        let body = match (method, url) {
            ("PROPFIND", "https://dav.example.com/") => PRINCIPAL,
            ("PROPFIND", "https://dav.example.com/principals/olibuijr/") => HOME,
            ("PROPFIND", "https://dav.example.com/calendars/olibuijr/") => COLLECTIONS,
            ("REPORT", "https://dav.example.com/calendars/olibuijr/home/") => EVENTS,
            _ => return Ok((404, String::new())),
        };
        Ok((207, body.to_string()))
    }

    #[test]
    fn the_calendars_of_an_account_are_found_by_discovery() {
        let auth = basic_auth("olibuijr", "pw");
        let mut send = |req: &URLRequest| server(&req.method, &req.url);
        let cals = collections(&mut send, BASE, Kind::CalDav, &auth).unwrap();
        assert_eq!(cals, vec!["https://dav.example.com/calendars/olibuijr/home/".to_string()]);
    }

    #[test]
    fn events_come_back_as_their_calendar_text() {
        let auth = basic_auth("olibuijr", "pw");
        let mut send = |req: &URLRequest| server(&req.method, &req.url);
        let texts = calendar_texts(&mut send, BASE, &auth, 0, 86_400).unwrap();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].starts_with("BEGIN:VCALENDAR"));
    }

    #[test]
    fn a_refused_answer_is_an_error_with_its_status() {
        let auth = basic_auth("olibuijr", "pw");
        let mut send = |_: &URLRequest| -> Result<(u16, String), String> { Ok((401, String::new())) };
        let err = collections(&mut send, BASE, Kind::CalDav, &auth).unwrap_err();
        assert!(err.contains("principal") || err.contains("401"), "{err}");
    }

    #[test]
    fn the_basic_header_is_base64_of_user_and_password() {
        assert_eq!(basic_auth("user", "pass"), "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn a_request_carries_its_depth_and_login() {
        let req = request("PROPFIND", "https://x.example/", "Basic abc", "1", String::new());
        assert_eq!(req.method, "PROPFIND");
        assert!(req.headers.iter().any(|(k, v)| k == "Depth" && v == "1"));
        assert!(req.headers.iter().any(|(k, v)| k == "Authorization" && v == "Basic abc"));
    }
}
