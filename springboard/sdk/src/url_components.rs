//! `URLComponents` and `URLQueryItem` parity: `URLComponents::from_string` splits a URL into scheme, user, password,
//! host, port, path, query items and fragment. Every part can be edited, and `string()` builds the URL again.
//!
//! The parts are stored decoded, as `URLComponents` keeps them in its native properties; the `percent_encoded_*`
//! accessors give the RFC 3986 encoded form of each part. Pure std: no network, no URL crate. Decoding is lossy for
//! bytes that are not UTF-8. A `+` is an ordinary character in every part (form encoding is not applied), and an
//! empty query segment (`a=1&&b`) is skipped.

/// `URLQueryItem`: one name and an optional value from the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLQueryItem {
    pub name: String,
    pub value: Option<String>,
}

impl URLQueryItem {
    pub fn new(name: &str, value: Option<&str>) -> Self {
        URLQueryItem { name: name.to_string(), value: value.map(str::to_string) }
    }
}

/// `URLComponents`. Parts that the URL does not have are `None`; `path` is empty when there is none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct URLComponents {
    pub scheme: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    /// `Some("")` for an empty authority (`file:///tmp`); `None` when the URL has no `//` part.
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: String,
    /// `None` when the URL has no `?`; an empty list for a bare `?`.
    pub query_items: Option<Vec<URLQueryItem>>,
    pub fragment: Option<String>,
}

impl URLComponents {
    /// `init(string:)`. None when the port is not a number that fits in 16 bits or the authority is malformed.
    pub fn from_string(s: &str) -> Option<Self> {
        let mut c = URLComponents::default();
        let (rest, fragment) = match s.split_once('#') {
            Some((r, f)) => (r, Some(f)),
            None => (s, None),
        };
        c.fragment = fragment.map(percent_decode);
        let (rest, query) = match rest.split_once('?') {
            Some((r, q)) => (r, Some(q)),
            None => (rest, None),
        };
        c.query_items = query.map(parse_query);
        let rest = match scheme_end(rest) {
            Some(i) => {
                c.scheme = Some(rest[..i].to_string());
                &rest[i + 1..]
            }
            None => rest,
        };
        let path = match rest.strip_prefix("//") {
            Some(after) => {
                let (authority, path) = match after.find('/') {
                    Some(i) => (&after[..i], &after[i..]),
                    None => (after, ""),
                };
                parse_authority(authority, &mut c)?;
                path
            }
            None => rest,
        };
        c.path = percent_decode(path);
        Some(c)
    }

    /// `query`: the decoded query string (`a=1&b`), None when there is no query.
    pub fn query(&self) -> Option<String> {
        let items = self.query_items.as_ref()?;
        Some(
            items
                .iter()
                .map(|i| match &i.value {
                    Some(v) => format!("{}={}", i.name, v),
                    None => i.name.clone(),
                })
                .collect::<Vec<_>>()
                .join("&"),
        )
    }

    /// `percentEncodedPath`.
    pub fn percent_encoded_path(&self) -> String {
        encode(&self.path, is_path)
    }

    /// `percentEncodedQuery`: the encoded query string, None when there is no query.
    pub fn percent_encoded_query(&self) -> Option<String> {
        let items = self.query_items.as_ref()?;
        Some(
            items
                .iter()
                .map(|i| match &i.value {
                    Some(v) => format!("{}={}", encode(&i.name, is_query), encode(v, is_query)),
                    None => encode(&i.name, is_query),
                })
                .collect::<Vec<_>>()
                .join("&"),
        )
    }

    /// `percentEncodedUser`.
    pub fn percent_encoded_user(&self) -> Option<String> {
        self.user.as_deref().map(|u| encode(u, is_userinfo))
    }

    /// `percentEncodedPassword`.
    pub fn percent_encoded_password(&self) -> Option<String> {
        self.password.as_deref().map(|p| encode(p, is_userinfo))
    }

    /// `percentEncodedHost`. An IPv6 literal keeps its colons; brackets are added by `string()`.
    pub fn percent_encoded_host(&self) -> Option<String> {
        self.host.as_deref().map(|h| encode(h, is_host))
    }

    /// `percentEncodedFragment`.
    pub fn percent_encoded_fragment(&self) -> Option<String> {
        self.fragment.as_deref().map(|f| encode(f, is_fragment))
    }

    /// `string`: the URL built from the parts, with every part percent-encoded.
    pub fn string(&self) -> String {
        let mut out = String::new();
        if let Some(s) = &self.scheme {
            out.push_str(s);
            out.push(':');
        }
        let authority = self.host.is_some() || self.user.is_some() || self.password.is_some() || self.port.is_some();
        if authority {
            out.push_str("//");
            if self.user.is_some() || self.password.is_some() {
                out.push_str(&self.percent_encoded_user().unwrap_or_default());
                if let Some(p) = self.percent_encoded_password() {
                    out.push(':');
                    out.push_str(&p);
                }
                out.push('@');
            }
            if let Some(h) = &self.host {
                if h.contains(':') {
                    out.push('[');
                    out.push_str(h);
                    out.push(']');
                } else {
                    out.push_str(&encode(h, is_host));
                }
            }
            if let Some(p) = self.port {
                out.push_str(&format!(":{p}"));
            }
        }
        let path = self.percent_encoded_path();
        if authority && !path.is_empty() && !path.starts_with('/') {
            out.push('/');
        }
        out.push_str(&path);
        if let Some(q) = self.percent_encoded_query() {
            out.push('?');
            out.push_str(&q);
        }
        if let Some(f) = self.percent_encoded_fragment() {
            out.push('#');
            out.push_str(&f);
        }
        out
    }
}

/// The index of the `:` that ends a valid scheme (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`), if the text starts with one.
fn scheme_end(s: &str) -> Option<usize> {
    let i = s.find(':')?;
    let mut chars = s[..i].chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then_some(i)
}

/// Splits `user:password@host:port` (or `[v6]:port`) into `c`.
fn parse_authority(auth: &str, c: &mut URLComponents) -> Option<()> {
    let hostport = match auth.rsplit_once('@') {
        Some((info, hp)) => {
            match info.split_once(':') {
                Some((u, p)) => {
                    c.user = Some(percent_decode(u));
                    c.password = Some(percent_decode(p));
                }
                None => c.user = Some(percent_decode(info)),
            }
            hp
        }
        None => auth,
    };
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?),
        };
        (h, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    c.port = match port {
        None | Some("") => None,
        Some(p) => Some(p.parse().ok()?),
    };
    c.host = Some(percent_decode(host));
    Some(())
}

/// Splits a query on `&`; the first `=` separates the name from the value.
fn parse_query(q: &str) -> Vec<URLQueryItem> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((n, v)) => URLQueryItem { name: percent_decode(n), value: Some(percent_decode(v)) },
            None => URLQueryItem { name: percent_decode(pair), value: None },
        })
        .collect()
}

/// Decodes `%XX` escapes. A `%` that is not followed by two hex digits is kept as it is.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Percent-encodes every byte that `keep` rejects.
fn encode(s: &str, keep: fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if keep(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn sub_delims(b: u8) -> bool {
    matches!(b, b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=')
}

fn pchar(b: u8) -> bool {
    unreserved(b) || sub_delims(b) || matches!(b, b':' | b'@')
}

fn is_userinfo(b: u8) -> bool {
    unreserved(b) || sub_delims(b)
}

fn is_host(b: u8) -> bool {
    unreserved(b) || sub_delims(b) || b == b':'
}

fn is_path(b: u8) -> bool {
    pchar(b) || b == b'/'
}

/// A query name or value: `&`, `=` and `+` are encoded so that the separators stay unambiguous.
fn is_query(b: u8) -> bool {
    (pchar(b) && !matches!(b, b'&' | b'=' | b'+')) || b == b'/' || b == b'?'
}

fn is_fragment(b: u8) -> bool {
    pchar(b) || b == b'/' || b == b'?'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_part_and_decodes_them() {
        let c = URLComponents::from_string(
            "https://ann%40x:p%20w@example.com:8443/a%20b/c?q=1&r=x%26y#top%201",
        )
        .unwrap();
        assert_eq!(c.scheme.as_deref(), Some("https"));
        assert_eq!(c.user.as_deref(), Some("ann@x"));
        assert_eq!(c.password.as_deref(), Some("p w"));
        assert_eq!(c.host.as_deref(), Some("example.com"));
        assert_eq!(c.port, Some(8443));
        assert_eq!(c.path, "/a b/c");
        assert_eq!(c.query().as_deref(), Some("q=1&r=x&y"));
        assert_eq!(c.fragment.as_deref(), Some("top 1"));
    }

    #[test]
    fn query_items_keep_order_and_plus_is_literal() {
        let c = URLComponents::from_string("http://h/p?b=2&a&c=x+y&a=3").unwrap();
        let items = c.query_items.unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["b", "a", "c", "a"]);
        assert_eq!(items[1].value, None);
        assert_eq!(items[2].value.as_deref(), Some("x+y"));
    }

    #[test]
    fn absent_query_is_none_and_a_bare_question_mark_is_empty() {
        assert_eq!(URLComponents::from_string("http://h/").unwrap().query_items, None);
        assert_eq!(URLComponents::from_string("http://h/?").unwrap().query_items, Some(Vec::new()));
    }

    #[test]
    fn building_percent_encodes_items_and_path() {
        let mut c = URLComponents::from_string("https://example.com/search").unwrap();
        c.query_items = Some(vec![URLQueryItem::new("q", Some("a b&c=d")), URLQueryItem::new("flag", None)]);
        c.path = "/search results".into();
        assert_eq!(c.string(), "https://example.com/search%20results?q=a%20b%26c%3Dd&flag");
        assert_eq!(c.percent_encoded_query().as_deref(), Some("q=a%20b%26c%3Dd&flag"));
    }

    #[test]
    fn round_trips_a_canonical_url_including_an_ipv6_host() {
        let s = "https://user@[::1]:8080/a/b?x=1#frag";
        let c = URLComponents::from_string(s).unwrap();
        assert_eq!(c.host.as_deref(), Some("::1"));
        assert_eq!(c.port, Some(8080));
        assert_eq!(c.string(), s);
    }

    #[test]
    fn rejects_a_bad_port_and_keeps_an_empty_authority_host() {
        assert!(URLComponents::from_string("http://h:99999/").is_none());
        assert!(URLComponents::from_string("http://h:x/").is_none());
        assert_eq!(URLComponents::from_string("file:///tmp/a").unwrap().host.as_deref(), Some(""));
        assert_eq!(URLComponents::from_string("file:///tmp/a").unwrap().string(), "file:///tmp/a");
    }

    #[test]
    fn percent_encoded_accessors_encode_each_part() {
        let mut c = URLComponents::default();
        c.path = "/a b/\u{fc}".into();
        c.fragment = Some("x y".into());
        assert_eq!(c.percent_encoded_path(), "/a%20b/%C3%BC");
        assert_eq!(c.percent_encoded_fragment().as_deref(), Some("x%20y"));
        assert_eq!(c.percent_encoded_query(), None);
    }
}
