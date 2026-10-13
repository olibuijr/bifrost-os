//! Captive network assistant: after a network joins, one plain HTTP request to a known probe address tells
//! whether the network is open to the internet or sits behind a sign-in page. A portal answers the probe with
//! a redirect or a page of its own; the shell then opens that page in the browser and offers "Sign In" in the
//! Wi-Fi menu. The probe is one request per network and address, so a flaky link does not probe again and again.
//!
//! The reply classification and the bookkeeping are pure and unit tested; `probe` is the only part that touches
//! the network.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The probe answers `204 No Content` with an empty body when the network is open.
pub const PROBE_URL: &str = "http://connectivity.olibuijr.com/generate_204";
/// How long the connect, the send and the reply may each take.
pub const TIMEOUT: Duration = Duration::from_secs(4);
/// The reply is read up to this many bytes; a portal page is never needed in full.
const MAX_REPLY: u64 = 64 * 1024;

/// What the probe found.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// the probe got its own 204: open access
    Online,
    /// a sign-in page; the URL to open in the browser
    Portal(String),
    /// no answer, or an answer that is neither the probe's 204 nor a redirect or page
    Offline,
}

/// The parts of an HTTP/1.x reply the classification needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// Parses the status line, the `Location` header and the body of a reply. `None` when there is no status line.
pub fn parse_reply(buf: &[u8]) -> Option<Reply> {
    let (head, body) = match find(buf, b"\r\n\r\n") {
        Some(i) => (&buf[..i], &buf[i + 4..]),
        None => match find(buf, b"\n\n") {
            Some(i) => (&buf[..i], &buf[i + 2..]),
            None => (buf, &buf[buf.len()..]),
        },
    };
    let text = String::from_utf8_lossy(head);
    let mut lines = text.lines();
    let status_line = lines.next()?;
    let mut parts = status_line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;
    let location = lines.find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.trim().eq_ignore_ascii_case("location").then(|| value.trim().to_string())
    });
    Some(Reply { status, location, body: body.to_vec() })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splits a plain `http://host[:port][/path]` URL. Other schemes and IPv6 literals are not probed.
pub fn split_http_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('[') || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port, path))
}

/// The page a redirect points at: an absolute http(s) URL as is, a path against the probe's origin, else the
/// probe address itself.
pub fn redirect_target(probe_url: &str, location: Option<&str>) -> String {
    let Some(loc) = location.map(str::trim).filter(|l| !l.is_empty()) else { return probe_url.to_string() };
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    if loc.starts_with('/') {
        if let Some((host, port, _)) = split_http_url(probe_url) {
            let origin = if port == 80 { format!("http://{host}") } else { format!("http://{host}:{port}") };
            return format!("{origin}{loc}");
        }
    }
    probe_url.to_string()
}

/// Classifies the reply to the probe: 204 is open access, a redirect or any other answer is a sign-in page, and
/// no reply is offline. Server errors count as offline, since a failing probe says nothing about the network.
pub fn classify(probe_url: &str, reply: Option<&Reply>) -> Verdict {
    match reply {
        None => Verdict::Offline,
        Some(r) if r.status == 204 => Verdict::Online,
        Some(r) if (300..400).contains(&r.status) => Verdict::Portal(redirect_target(probe_url, r.location.as_deref())),
        Some(r) if (200..300).contains(&r.status) => Verdict::Portal(probe_url.to_string()),
        Some(_) => Verdict::Offline,
    }
}

/// Sends the probe and classifies the answer. Blocking: call it from a thread, never from the shell's loop.
pub fn probe(url: &str, timeout: Duration) -> Verdict {
    let Some((host, port, path)) = split_http_url(url) else { return Verdict::Offline };
    let Some(addr) = (host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next()) else { return Verdict::Offline };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else { return Verdict::Offline };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: BifrOSt\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return Verdict::Offline;
    }
    let mut buf = Vec::new();
    let _ = (&mut stream).take(MAX_REPLY).read_to_end(&mut buf);
    classify(url, parse_reply(&buf).as_ref())
}

/// What the shell keeps about the current network: which link is being probed, the sign-in page if the link has
/// one, and the page to open once. Keys are the caller's (the SSID and address), so a new network starts over.
#[derive(Debug, Default)]
pub struct Assistant {
    key: Option<String>,
    portal: Option<String>,
    open: Option<String>,
    opened_for: Option<String>,
}

impl Assistant {
    /// Called on every status tick with the current link (None when offline). Returns the key to probe when
    /// the link is new; a link already seen is not probed again.
    pub fn begin(&mut self, link: Option<String>) -> Option<String> {
        let Some(key) = link else {
            *self = Assistant::default();
            return None;
        };
        if self.key.as_deref() == Some(key.as_str()) {
            return None;
        }
        self.key = Some(key.clone());
        self.portal = None;
        self.open = None;
        Some(key)
    }

    /// The probe for `key` finished. Ignored when the link has changed since it started.
    pub fn finish(&mut self, key: &str, verdict: Verdict) {
        if self.key.as_deref() != Some(key) {
            return;
        }
        match verdict {
            Verdict::Portal(url) => {
                if self.opened_for.as_deref() != Some(key) {
                    self.opened_for = Some(key.to_string());
                    self.open = Some(url.clone());
                }
                self.portal = Some(url);
            }
            Verdict::Online | Verdict::Offline => self.portal = None,
        }
    }

    /// The sign-in page of the current network, while it has one.
    pub fn portal(&self) -> Option<&str> {
        self.portal.as_deref()
    }

    /// The sign-in page to open now, once per network.
    pub fn take_open(&mut self) -> Option<String> {
        self.open.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, location: Option<&str>) -> Reply {
        Reply { status, location: location.map(str::to_string), body: Vec::new() }
    }

    #[test]
    fn a_204_is_an_open_network() {
        let r = parse_reply(b"HTTP/1.1 204 No Content\r\nDate: x\r\n\r\n").unwrap();
        assert_eq!(classify(PROBE_URL, Some(&r)), Verdict::Online);
    }

    #[test]
    fn a_redirect_to_a_sign_in_page_is_a_portal_at_that_page() {
        let r = parse_reply(b"HTTP/1.1 302 Found\r\nLocation: https://portal.example.net/login?x=1\r\n\r\n").unwrap();
        assert_eq!(classify(PROBE_URL, Some(&r)), Verdict::Portal("https://portal.example.net/login?x=1".into()));
    }

    #[test]
    fn a_relative_redirect_resolves_against_the_probe_host() {
        assert_eq!(redirect_target(PROBE_URL, Some("/login")), "http://connectivity.olibuijr.com/login");
        assert_eq!(redirect_target("http://probe.test:8080/g", Some("/x")), "http://probe.test:8080/x");
    }

    #[test]
    fn a_redirect_without_a_location_or_with_a_bad_one_opens_the_probe_itself() {
        assert_eq!(redirect_target(PROBE_URL, None), PROBE_URL);
        assert_eq!(redirect_target(PROBE_URL, Some("javascript:alert(1)")), PROBE_URL);
        assert_eq!(classify(PROBE_URL, Some(&reply(302, None))), Verdict::Portal(PROBE_URL.into()));
    }

    #[test]
    fn an_intercepted_page_with_200_is_a_portal() {
        let r = parse_reply(b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n\r\n<html>Sign in</html>").unwrap();
        assert_eq!(r.body, b"<html>Sign in</html>".to_vec());
        assert_eq!(classify(PROBE_URL, Some(&r)), Verdict::Portal(PROBE_URL.into()));
    }

    #[test]
    fn server_errors_and_silence_are_offline() {
        assert_eq!(classify(PROBE_URL, None), Verdict::Offline);
        assert_eq!(classify(PROBE_URL, Some(&reply(503, None))), Verdict::Offline);
        assert_eq!(parse_reply(b"garbage"), None);
        assert_eq!(parse_reply(b""), None);
    }

    #[test]
    fn header_names_are_case_insensitive_and_bare_newlines_are_accepted() {
        let r = parse_reply(b"HTTP/1.0 302 Found\nlocation: /in\n\n").unwrap();
        assert_eq!(r.location.as_deref(), Some("/in"));
        assert_eq!(r.status, 302);
    }

    #[test]
    fn plain_http_urls_split_into_host_port_and_path() {
        assert_eq!(split_http_url(PROBE_URL), Some(("connectivity.olibuijr.com".into(), 80, "/generate_204".into())));
        assert_eq!(split_http_url("http://10.0.0.1:8080"), Some(("10.0.0.1".into(), 8080, "/".into())));
        assert_eq!(split_http_url("https://secure.example/"), None);
        assert_eq!(split_http_url("http://[::1]/"), None);
        assert_eq!(split_http_url("http://:80/"), None);
        assert_eq!(split_http_url("http://host:notaport/"), None);
    }

    #[test]
    fn a_probe_of_an_unusable_url_is_offline_without_touching_the_network() {
        assert_eq!(probe("https://example.invalid/", Duration::from_millis(50)), Verdict::Offline);
    }

    #[test]
    fn the_assistant_probes_each_new_link_once_and_opens_a_portal_once() {
        let mut a = Assistant::default();
        assert_eq!(a.begin(Some("Cafe|192.168.1.5".into())), Some("Cafe|192.168.1.5".into()));
        assert_eq!(a.begin(Some("Cafe|192.168.1.5".into())), None, "the same link is not probed again");
        a.finish("Cafe|192.168.1.5", Verdict::Portal("http://p.test/login".into()));
        assert_eq!(a.portal(), Some("http://p.test/login"));
        assert_eq!(a.take_open(), Some("http://p.test/login".into()));
        assert_eq!(a.take_open(), None, "opened once");
        assert_eq!(a.begin(Some("Cafe|192.168.1.5".into())), None);
        assert_eq!(a.portal(), Some("http://p.test/login"), "the menu keeps the sign-in row");
    }

    #[test]
    fn a_late_answer_for_an_old_link_is_ignored() {
        let mut a = Assistant::default();
        a.begin(Some("A|1".into()));
        a.begin(Some("B|2".into()));
        a.finish("A|1", Verdict::Portal("http://old.test".into()));
        assert_eq!(a.portal(), None);
        assert_eq!(a.take_open(), None);
    }

    #[test]
    fn going_offline_forgets_the_portal_and_a_return_to_the_same_network_opens_it_again() {
        let mut a = Assistant::default();
        a.begin(Some("Cafe|1".into()));
        a.finish("Cafe|1", Verdict::Portal("http://p.test".into()));
        a.take_open();
        assert_eq!(a.begin(None), None);
        assert_eq!(a.portal(), None);
        assert_eq!(a.begin(Some("Cafe|1".into())), Some("Cafe|1".into()));
        a.finish("Cafe|1", Verdict::Portal("http://p.test".into()));
        assert_eq!(a.take_open(), Some("http://p.test".into()));
    }

    #[test]
    fn an_open_network_clears_a_portal_row() {
        let mut a = Assistant::default();
        a.begin(Some("Home|1".into()));
        a.finish("Home|1", Verdict::Portal("http://p.test".into()));
        a.finish("Home|1", Verdict::Online);
        assert_eq!(a.portal(), None);
    }
}
