//! CalDAV and CardDAV on the wire: the discovery URLs, the request bodies and the reading of multistatus answers.
//! The transport is the SDK's URL session; this module only builds requests and reads answers, so it runs on the host.
//!
//! Discovery (RFC 6764) starts at the well-known path of the server, asks for the current user's principal, then for
//! the home set of that kind, then lists the collections under it. A calendar collection is `resourcetype` with
//! `calendar`; an address book is `addressbook`.

use super::Kind;

pub const DAV_NS: &str = "DAV:";
pub const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
pub const CARDDAV_NS: &str = "urn:ietf:params:xml:ns:carddav";

/// The largest answer read. A bigger one is refused rather than parsed.
pub const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;

/// The well-known path that starts discovery for `kind`.
pub fn well_known_path(kind: Kind) -> &'static str {
    match kind {
        Kind::CardDav => "/.well-known/carddav",
        _ => "/.well-known/caldav",
    }
}

/// The `scheme://authority` part of a URL, or `None` when it has no scheme.
pub fn origin(url: &str) -> Option<String> {
    let scheme_end = url.find("://")? + 3;
    let rest = &url[scheme_end..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(format!("{}{}", &url[..scheme_end], &rest[..end]))
}

/// Resolves an `href` from an answer against the URL the request was sent to.
pub fn resolve(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{}{}", origin(base).unwrap_or_default(), href)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), href)
    }
}

/// The body that asks for the current user's principal URL.
pub fn principal_body() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="{DAV_NS}"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#
    )
}

/// The body that asks a principal for the home set of `kind`: `calendar-home-set` or `addressbook-home-set`.
pub fn home_set_body(kind: Kind) -> String {
    match kind {
        Kind::CardDav => format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="{DAV_NS}" xmlns:card="{CARDDAV_NS}"><d:prop><card:addressbook-home-set/></d:prop></d:propfind>"#
        ),
        _ => format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="{DAV_NS}" xmlns:c="{CALDAV_NS}"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#
        ),
    }
}

/// The body that lists a home set's collections (send with `Depth: 1`).
pub fn collections_body() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="{DAV_NS}"><d:prop><d:displayname/><d:resourcetype/><d:getctag/></d:prop></d:propfind>"#
    )
}

/// The REPORT that returns the events of one calendar that overlap `start..end` (Unix seconds), with their text.
pub fn calendar_query_body(start: i64, end: i64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="{DAV_NS}" xmlns:c="{CALDAV_NS}"><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{}" end="{}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
        utc_stamp(start),
        utc_stamp(end)
    )
}

/// The REPORT that returns every card of one address book, with their text.
pub fn addressbook_query_body() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><card:addressbook-query xmlns:d="{DAV_NS}" xmlns:card="{CARDDAV_NS}"><d:prop><d:getetag/><card:address-data/></d:prop></card:addressbook-query>"#
    )
}

/// A UTC time in the iCalendar form `YYYYMMDDTHHMMSSZ`.
pub fn utc_stamp(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", y, m, d, secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Year, month and day of a day number counted from 1970-01-01 (proleptic Gregorian calendar).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// One `response` of a multistatus answer: its `href`, and the properties of its successful `propstat` blocks.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Response {
    pub href: String,
    /// (local property name, value). A value with child elements is the text of those children, one per line, or the
    /// child names when they have no text (a `resourcetype` gives `collection calendar`).
    pub props: Vec<(String, String)>,
}

impl Response {
    pub fn prop(&self, name: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// The kind of collection a response describes, from its `resourcetype`: `calendar` or `addressbook`.
pub fn collection_kind(r: &Response) -> Option<Kind> {
    let types = r.prop("resourcetype")?;
    let has = |token: &str| types.split_whitespace().any(|t| t == token);
    if has("calendar") {
        Some(Kind::CalDav)
    } else if has("addressbook") {
        Some(Kind::CardDav)
    } else {
        None
    }
}

/// The URLs in a value that holds `href`s, one per line (such as `current-user-principal`).
pub fn hrefs(value: &str) -> Vec<&str> {
    value.lines().map(str::trim).filter(|v| !v.is_empty()).collect()
}

/// Reads a multistatus answer. Properties in a `propstat` whose status is not 200 are left out.
pub fn parse_multistatus(xml: &str) -> Vec<Response> {
    let arena = parse_tree(xml);
    let mut out = Vec::new();
    for (i, el) in arena.iter().enumerate() {
        if el.name == "response" {
            out.push(response_of(&arena, i));
        }
    }
    out
}

struct El {
    name: String,
    text: String,
    children: Vec<usize>,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn response_of(arena: &[El], idx: usize) -> Response {
    let mut r = Response::default();
    for &c in &arena[idx].children {
        let el = &arena[c];
        match el.name.as_str() {
            "href" => r.href = el.text.trim().to_string(),
            "propstat" => {
                let ok = arena[c]
                    .children
                    .iter()
                    .any(|&s| arena[s].name == "status" && arena[s].text.split_whitespace().any(|w| w == "200"));
                if !ok {
                    continue;
                }
                for &p in &el.children {
                    if arena[p].name != "prop" {
                        continue;
                    }
                    for &prop in &arena[p].children {
                        r.props.push((arena[prop].name.clone(), value_of(arena, prop)));
                    }
                }
            }
            _ => {}
        }
    }
    r
}

fn value_of(arena: &[El], idx: usize) -> String {
    let el = &arena[idx];
    if el.children.is_empty() {
        return el.text.trim().to_string();
    }
    let texts: Vec<&str> = el.children.iter().map(|&c| arena[c].text.trim()).filter(|t| !t.is_empty()).collect();
    if !texts.is_empty() {
        return texts.join("\n");
    }
    el.children.iter().map(|&c| arena[c].name.as_str()).collect::<Vec<_>>().join(" ")
}

/// Builds an arena of elements from XML text. Index 0 is a virtual root that holds text outside any element.
fn parse_tree(xml: &str) -> Vec<El> {
    let mut arena = vec![El { name: String::new(), text: String::new(), children: Vec::new() }];
    let mut stack = vec![0usize];
    let mut rest = xml;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.split_once("-->").map(|(_, r)| r).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let (body, r) = after.split_once("]]>").unwrap_or((after, ""));
            let top = *stack.last().unwrap_or(&0);
            arena[top].text.push_str(body);
            rest = r;
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = rest.split_once('>').map(|(_, r)| r).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("</") {
            let (name, r) = after.split_once('>').unwrap_or((after, ""));
            let local = local_name(name.trim());
            if let Some(pos) = stack.iter().rposition(|&i| i != 0 && arena[i].name == local) {
                stack.truncate(pos);
            }
            rest = r;
        } else if let Some(after) = rest.strip_prefix('<') {
            let (tag, r) = after.split_once('>').unwrap_or((after, ""));
            let self_closing = tag.trim_end().ends_with('/');
            let body = tag.trim().trim_end_matches('/').trim_end();
            let name = body.split_whitespace().next().unwrap_or("");
            let parent = *stack.last().unwrap_or(&0);
            let idx = arena.len();
            arena.push(El { name: local_name(name).to_string(), text: String::new(), children: Vec::new() });
            arena[parent].children.push(idx);
            if !self_closing {
                stack.push(idx);
            }
            rest = r;
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let top = *stack.last().unwrap_or(&0);
            arena[top].text.push_str(&unescape(&rest[..end]));
            rest = &rest[end..];
        }
    }
    arena
}

/// Decodes the five predefined XML entities and numeric character references. Unknown ones stay as written.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let ent = &after[..semi];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent.strip_prefix('#').and_then(|n| match n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
            };
            c.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
 <d:response>
  <d:href>/calendars/olibuijr/</d:href>
  <d:propstat>
   <d:prop>
    <d:displayname>Home &amp; Work</d:displayname>
    <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
   </d:prop>
   <d:status>HTTP/1.1 200 OK</d:status>
  </d:propstat>
  <d:propstat>
   <d:prop><d:getctag/></d:prop>
   <d:status>HTTP/1.1 404 Not Found</d:status>
  </d:propstat>
 </d:response>
</d:multistatus>"#;

    #[test]
    fn a_calendar_collection_is_read_with_its_name() {
        let got = parse_multistatus(HOME);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].href, "/calendars/olibuijr/");
        assert_eq!(got[0].prop("displayname"), Some("Home & Work"));
        assert_eq!(collection_kind(&got[0]), Some(Kind::CalDav));
        assert_eq!(got[0].prop("getctag"), None, "a 404 propstat is left out");
    }

    #[test]
    fn a_principal_href_is_the_text_of_its_child() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop>
            <d:current-user-principal><d:href>/principals/olibuijr/</d:href></d:current-user-principal>
            </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let got = parse_multistatus(xml);
        assert_eq!(hrefs(got[0].prop("current-user-principal").unwrap()), vec!["/principals/olibuijr/"]);
    }

    #[test]
    fn an_address_book_collection_is_recognised() {
        let r = Response { href: "/ab/".into(), props: vec![("resourcetype".into(), "collection addressbook".into())] };
        assert_eq!(collection_kind(&r), Some(Kind::CardDav));
        let plain = Response { href: "/x/".into(), props: vec![("resourcetype".into(), "collection".into())] };
        assert_eq!(collection_kind(&plain), None);
    }

    #[test]
    fn calendar_data_keeps_its_lines_and_escapes() {
        let xml = "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:response><d:href>/c/a.ics</d:href>\
            <d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR\r\nSUMMARY:a &amp; b\r\nEND:VCALENDAR</c:calendar-data></d:prop>\
            <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";
        let got = parse_multistatus(xml);
        assert_eq!(got[0].prop("calendar-data"), Some("BEGIN:VCALENDAR\r\nSUMMARY:a & b\r\nEND:VCALENDAR"));
    }

    #[test]
    fn utc_stamps_are_compact_iso_times() {
        assert_eq!(utc_stamp(1_700_000_000), "20231114T221320Z");
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
    }

    #[test]
    fn hrefs_resolve_against_the_request_url() {
        assert_eq!(origin("https://dav.example.com:8443/a/b").as_deref(), Some("https://dav.example.com:8443"));
        assert_eq!(resolve("https://dav.example.com/a/", "/cal/x/"), "https://dav.example.com/cal/x/");
        assert_eq!(resolve("https://dav.example.com/a/", "https://other.example/y/"), "https://other.example/y/");
        assert_eq!(well_known_path(Kind::CardDav), "/.well-known/carddav");
        assert_eq!(well_known_path(Kind::CalDav), "/.well-known/caldav");
    }

    #[test]
    fn query_bodies_name_their_namespaces_and_window() {
        let q = calendar_query_body(1_700_000_000, 1_700_086_400);
        assert!(q.contains(r#"start="20231114T221320Z""#), "{q}");
        assert!(q.contains(r#"name="VEVENT""#));
        assert!(addressbook_query_body().contains("address-data"));
        assert!(home_set_body(Kind::CardDav).contains("addressbook-home-set"));
        assert!(home_set_body(Kind::CalDav).contains("calendar-home-set"));
    }

    #[test]
    fn a_bad_entity_is_kept_as_written() {
        assert_eq!(unescape("a &bogus; &#65;&#x42; &"), "a &bogus; AB &");
    }
}
