//! `HTTPCookie` and `HTTPCookieStorage` parity for the URL loading system: parses `Set-Cookie` values, stores cookies
//! per host, path and secure flag, and builds the `Cookie` request header for a URL.
//!
//! Pure std, no network. The clock is passed in as Unix seconds (`now`), so expiry is testable. Only `Max-Age` sets an
//! expiry: `Expires` dates are ignored, so such a cookie behaves as a session cookie (a gap, see `docs/parity/w3-cfnetwork.md`).
//! Domain matching follows RFC 6265 section 5.1.3 without a public-suffix list.

use crate::url_components::URLComponents;

/// `HTTPCookie.AcceptPolicy`: the storage keeps cookies only when the policy allows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptPolicy {
    Always,
    Never,
}

/// One cookie. `domain` is lower case with no leading dot. A `host_only` cookie matches that host alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HTTPCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Expiry in Unix seconds. `None` for a session cookie.
    pub expires: Option<u64>,
    /// Time the cookie was received, in Unix seconds.
    pub created: u64,
}

impl HTTPCookie {
    /// Parses one `Set-Cookie` value received from `url` at time `now`. Returns `None` when the name is empty, the URL
    /// has no host, or the `Domain` attribute does not cover the host.
    pub fn parse_set_cookie(header: &str, url: &str, now: u64) -> Option<HTTPCookie> {
        let parts = URLComponents::from_string(url)?;
        let host = parts.host.as_deref()?.to_ascii_lowercase();
        let mut fields = header.split(';');
        let (name, value) = fields.next()?.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut cookie = HTTPCookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: host.clone(),
            host_only: true,
            path: default_path(&parts.path),
            secure: false,
            http_only: false,
            expires: None,
            created: now,
        };
        let mut max_age: Option<i64> = None;
        for field in fields {
            let (key, val) = match field.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => (field.trim(), ""),
            };
            match key.to_ascii_lowercase().as_str() {
                "domain" => {
                    let domain = val.trim_start_matches('.').to_ascii_lowercase();
                    if domain.is_empty() {
                        continue;
                    }
                    if !domain_matches(&host, &domain) {
                        return None;
                    }
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
                "path" if val.starts_with('/') => cookie.path = val.to_string(),
                "max-age" => max_age = val.parse::<i64>().ok(),
                "secure" => cookie.secure = true,
                "httponly" => cookie.http_only = true,
                _ => {}
            }
        }
        // Max-Age wins over Expires in RFC 6265, and Expires is not parsed here.
        if let Some(age) = max_age {
            cookie.expires = Some(if age <= 0 { 0 } else { now.saturating_add(age as u64) });
        }
        Some(cookie)
    }

    /// True when the cookie has expired at `now`.
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires.map_or(false, |e| e <= now)
    }
}

/// Default cookie path (RFC 6265 section 5.1.4): the request path up to its last `/`.
fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => request_path[..i].to_string(),
    }
}

/// Domain match: the host is the domain, or ends with `.` plus the domain.
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).map_or(false, |p| p.ends_with('.'))
}

/// Path match (RFC 6265 section 5.1.4): equal, or a prefix that ends at a `/` boundary.
fn path_matches(request: &str, cookie: &str) -> bool {
    if request == cookie {
        return true;
    }
    request.starts_with(cookie) && (cookie.ends_with('/') || request[cookie.len()..].starts_with('/'))
}

/// `HTTPCookieStorage`: the cookies a client keeps. Cookies are kept in the order they were stored.
#[derive(Clone, Debug)]
pub struct HTTPCookieStorage {
    cookies: Vec<HTTPCookie>,
    pub cookie_accept_policy: AcceptPolicy,
}

impl HTTPCookieStorage {
    pub fn new() -> Self {
        HTTPCookieStorage { cookies: Vec::new(), cookie_accept_policy: AcceptPolicy::Always }
    }

    /// `setCookie(_:)`: stores a cookie, replacing one with the same name, domain and path. A cookie that is already
    /// expired at its receive time removes the stored one and is not kept. Returns `false` when the policy refuses cookies.
    pub fn set_cookie(&mut self, cookie: HTTPCookie) -> bool {
        if self.cookie_accept_policy == AcceptPolicy::Never {
            return false;
        }
        self.cookies.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        if !cookie.is_expired(cookie.created) {
            self.cookies.push(cookie);
        }
        true
    }

    /// Stores the `Set-Cookie` headers of a response from `url` received at `now`. Returns how many were accepted.
    pub fn set_cookies_from_headers(&mut self, url: &str, headers: &[(String, String)], now: u64) -> usize {
        let mut accepted = 0;
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            if let Some(cookie) = HTTPCookie::parse_set_cookie(value, url, now) {
                if self.set_cookie(cookie) {
                    accepted += 1;
                }
            }
        }
        accepted
    }

    /// `cookies`: every stored cookie, in storage order.
    pub fn cookies(&self) -> &[HTTPCookie] {
        &self.cookies
    }

    /// `cookies(for:)`: the unexpired cookies sent with a request to `url` at `now`. Ordered as RFC 6265 section 5.4
    /// asks: longer paths first, then older cookies first.
    pub fn cookies_for(&self, url: &str, now: u64) -> Vec<&HTTPCookie> {
        let parts = match URLComponents::from_string(url) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let host = match parts.host.as_deref() {
            Some(h) => h.to_ascii_lowercase(),
            None => return Vec::new(),
        };
        let https = parts.scheme.as_deref().map_or(false, |s| s.eq_ignore_ascii_case("https"));
        let path = if parts.path.is_empty() { "/" } else { parts.path.as_str() };
        let mut out: Vec<&HTTPCookie> = self
            .cookies
            .iter()
            .filter(|c| {
                let host_ok = (c.host_only && host == c.domain) || (!c.host_only && domain_matches(&host, &c.domain));
                !c.is_expired(now) && (!c.secure || https) && host_ok && path_matches(path, &c.path)
            })
            .collect();
        // sort_by is stable, so equal path lengths keep storage order.
        out.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        out
    }

    /// The `Cookie` request header for `url` at `now`, or `None` when no cookie matches.
    pub fn cookie_header(&self, url: &str, now: u64) -> Option<String> {
        let pairs: Vec<String> = self.cookies_for(url, now).iter().map(|c| format!("{}={}", c.name, c.value)).collect();
        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    /// `deleteCookie(_:)`: removes the cookie with the same name, domain and path. Returns whether one was removed.
    pub fn delete_cookie(&mut self, cookie: &HTTPCookie) -> bool {
        let before = self.cookies.len();
        self.cookies.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        self.cookies.len() != before
    }

    /// `removeCookies(since:)`: removes the cookies received at or after `since`.
    pub fn remove_cookies_since(&mut self, since: u64) {
        self.cookies.retain(|c| c.created < since);
    }

    /// Removes the cookies expired at `now` and returns how many were removed.
    pub fn purge_expired(&mut self, now: u64) -> usize {
        let before = self.cookies.len();
        self.cookies.retain(|c| !c.is_expired(now));
        before - self.cookies.len()
    }
}

impl Default for HTTPCookieStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(value: &str) -> Vec<(String, String)> {
        vec![("Set-Cookie".to_string(), value.to_string())]
    }

    #[test]
    fn parses_attributes_of_a_host_only_cookie() {
        let c = HTTPCookie::parse_set_cookie("sid=abc; Path=/shop; HttpOnly; Secure", "https://www.example.com/shop/cart", 1000)
            .unwrap();
        assert_eq!(c.name, "sid");
        assert_eq!(c.value, "abc");
        assert_eq!(c.domain, "www.example.com");
        assert!(c.host_only);
        assert_eq!(c.path, "/shop");
        assert!(c.secure && c.http_only);
        assert_eq!(c.expires, None);
        assert_eq!(c.created, 1000);
    }

    #[test]
    fn default_path_is_the_request_directory() {
        let c = HTTPCookie::parse_set_cookie("a=1", "https://example.com/x/y/z", 0).unwrap();
        assert_eq!(c.path, "/x/y");
        let root = HTTPCookie::parse_set_cookie("a=1", "https://example.com/z", 0).unwrap();
        assert_eq!(root.path, "/");
        let bare = HTTPCookie::parse_set_cookie("a=1", "https://example.com", 0).unwrap();
        assert_eq!(bare.path, "/");
    }

    #[test]
    fn domain_attribute_widens_the_cookie_or_is_rejected() {
        let wide = HTTPCookie::parse_set_cookie("u=1; Domain=.Example.com", "https://www.example.com/", 0).unwrap();
        assert_eq!(wide.domain, "example.com");
        assert!(!wide.host_only);
        assert_eq!(HTTPCookie::parse_set_cookie("u=1; Domain=evil.com", "https://www.example.com/", 0), None);
        assert_eq!(HTTPCookie::parse_set_cookie("=x", "https://example.com/", 0), None);
    }

    #[test]
    fn max_age_sets_expiry_and_zero_expires_at_once() {
        let c = HTTPCookie::parse_set_cookie("t=1; Max-Age=60", "https://example.com/", 1000).unwrap();
        assert_eq!(c.expires, Some(1060));
        assert!(!c.is_expired(1059));
        assert!(c.is_expired(1060));
        let gone = HTTPCookie::parse_set_cookie("t=1; Max-Age=0", "https://example.com/", 1000).unwrap();
        assert!(gone.is_expired(1000));
    }

    #[test]
    fn header_orders_longer_paths_first_and_filters_secure_and_path() {
        let mut jar = HTTPCookieStorage::new();
        let headers = vec![
            ("Set-Cookie".to_string(), "a=1; Path=/shop".to_string()),
            ("set-cookie".to_string(), "b=2; Path=/".to_string()),
            ("Set-Cookie".to_string(), "s=3; Secure; Path=/".to_string()),
        ];
        assert_eq!(jar.set_cookies_from_headers("https://example.com/", &headers, 1000), 3);
        assert_eq!(jar.cookie_header("https://example.com/shop/item", 1000), Some("a=1; b=2; s=3".to_string()));
        assert_eq!(jar.cookie_header("http://example.com/shop/item", 1000), Some("a=1; b=2".to_string()));
        assert_eq!(jar.cookie_header("https://example.com/other", 1000), Some("b=2; s=3".to_string()));
        assert_eq!(jar.cookie_header("https://other.com/", 1000), None);
    }

    #[test]
    fn host_only_cookie_stays_on_its_host_and_domain_cookie_covers_subdomains() {
        let mut jar = HTTPCookieStorage::new();
        jar.set_cookies_from_headers("https://example.com/", &hdr("h=1"), 1000);
        jar.set_cookies_from_headers("https://www.example.com/", &hdr("d=2; Domain=example.com"), 1000);
        assert_eq!(jar.cookie_header("https://example.com/", 1000), Some("h=1; d=2".to_string()));
        assert_eq!(jar.cookie_header("https://www.example.com/", 1000), Some("d=2".to_string()));
        assert_eq!(jar.cookie_header("https://a.example.com/", 1000), Some("d=2".to_string()));
    }

    #[test]
    fn expired_cookie_stops_matching_and_an_expiring_update_removes_it() {
        let mut jar = HTTPCookieStorage::new();
        jar.set_cookies_from_headers("https://example.com/", &hdr("k=1; Max-Age=10"), 1000);
        assert_eq!(jar.cookie_header("https://example.com/", 1009), Some("k=1".to_string()));
        assert_eq!(jar.cookie_header("https://example.com/", 1010), None);
        jar.set_cookies_from_headers("https://example.com/", &hdr("k=2; Max-Age=0"), 1005);
        assert!(jar.cookies().is_empty());
    }

    #[test]
    fn remove_since_delete_and_purge_work_on_receive_time() {
        let mut jar = HTTPCookieStorage::new();
        jar.set_cookies_from_headers("https://example.com/", &hdr("x=1"), 1000);
        jar.set_cookies_from_headers("https://example.com/", &hdr("y=2"), 2000);
        jar.remove_cookies_since(1500);
        assert_eq!(jar.cookies().len(), 1);
        assert_eq!(jar.cookies()[0].name, "x");
        let x = jar.cookies()[0].clone();
        assert!(jar.delete_cookie(&x));
        assert!(!jar.delete_cookie(&x));
        jar.set_cookies_from_headers("https://example.com/", &hdr("z=3; Max-Age=5"), 100);
        assert_eq!(jar.purge_expired(105), 1);
        assert!(jar.cookies().is_empty());
    }

    #[test]
    fn accept_policy_never_refuses_every_cookie() {
        let mut jar = HTTPCookieStorage::new();
        jar.cookie_accept_policy = AcceptPolicy::Never;
        let c = HTTPCookie::parse_set_cookie("a=1", "https://example.com/", 0).unwrap();
        assert!(!jar.set_cookie(c));
        assert!(jar.cookies().is_empty());
    }
}
