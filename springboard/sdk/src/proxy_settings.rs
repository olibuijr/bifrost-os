//! System proxy settings: which proxy an app's HTTP request goes through, and which hosts skip it. This is the
//! model behind `CFNetworkCopySystemProxySettings`: the shell publishes the proxy in the usual environment variables
//! (`http_proxy`, `https_proxy`, `no_proxy`, and their upper-case forms), and an app reads them through
//! [`ProxySettings::from_env`] and asks [`ProxySettings::proxy_for`] for each request it makes.
//!
//! The bypass list follows the common `no_proxy` conventions: `*` bypasses every host, an entry matches the host
//! exactly, and an entry matches its subdomains too when it starts with a dot (a leading dot on a plain entry is
//! ignored, so `example.com` and `.example.com` both match `a.example.com`). Host comparison ignores case. Address
//! ranges (CIDR) are not parsed; an entry that is an address matches only that exact address.

/// One proxy server: the host name or address and its port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyServer {
    pub host: String,
    pub port: u16,
}

/// Why a proxy URL could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyError {
    /// The value is empty or has no host.
    Empty,
    /// The scheme is not one a proxy URL can carry (only `http` and `https` are accepted, or none).
    BadScheme(String),
    /// The port is not a number from 1 to 65535.
    BadPort(String),
}

/// The default port of a proxy written without one.
pub const DEFAULT_PROXY_PORT: u16 = 80;

impl ProxyServer {
    /// Reads `http://host:port`, `https://host:port`, `host:port` or `host`. A missing port is
    /// [`DEFAULT_PROXY_PORT`]. A trailing slash is ignored. An IPv6 address is written in brackets.
    pub fn parse(value: &str) -> Result<ProxyServer, ProxyError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ProxyError::Empty);
        }
        let rest = match value.find("://") {
            Some(at) => {
                let scheme = &value[..at];
                if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
                    return Err(ProxyError::BadScheme(scheme.to_string()));
                }
                &value[at + 3..]
            }
            None => value,
        };
        // Anything after the authority is a path, which a proxy URL may end with (`http://host:3128/`).
        let rest = rest.split('/').next().unwrap_or("");
        if rest.is_empty() {
            return Err(ProxyError::Empty);
        }
        let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
            // [v6] or [v6]:port
            let close = inner.find(']').ok_or_else(|| ProxyError::BadPort(rest.to_string()))?;
            let host = &inner[..close];
            let after = &inner[close + 1..];
            let port = match after.strip_prefix(':') {
                Some(p) => p,
                None if after.is_empty() => "",
                None => return Err(ProxyError::BadPort(after.to_string())),
            };
            (host, port)
        } else {
            match rest.rfind(':') {
                Some(at) => (&rest[..at], &rest[at + 1..]),
                None => (rest, ""),
            }
        };
        if host.is_empty() {
            return Err(ProxyError::Empty);
        }
        let port = if port.is_empty() {
            DEFAULT_PROXY_PORT
        } else {
            match port.parse::<u16>() {
                Ok(p) if p != 0 => p,
                _ => return Err(ProxyError::BadPort(port.to_string())),
            }
        };
        Ok(ProxyServer { host: host.to_ascii_lowercase(), port })
    }
}

/// The proxies and bypass list the system is configured with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxySettings {
    /// The proxy for `http` requests.
    pub http: Option<ProxyServer>,
    /// The proxy for `https` requests.
    pub https: Option<ProxyServer>,
    /// Host entries that skip the proxy, in the order they were written. `*` is kept as written.
    pub no_proxy: Vec<String>,
}

impl ProxySettings {
    /// Reads the settings from the environment. `get` looks up one variable and returns its value, so the caller
    /// passes `|k| std::env::var(k).ok()` and a test passes a fixed map. The lower-case name wins over the
    /// upper-case one, as curl does. A proxy value that does not parse is treated as not set.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> ProxySettings {
        let pick = |lower: &str, upper: &str| get(lower).filter(|v| !v.trim().is_empty()).or_else(|| get(upper));
        let http = pick("http_proxy", "HTTP_PROXY").and_then(|v| ProxyServer::parse(&v).ok());
        let https = pick("https_proxy", "HTTPS_PROXY").and_then(|v| ProxyServer::parse(&v).ok());
        let no_proxy = pick("no_proxy", "NO_PROXY")
            .map(|v| parse_no_proxy(&v))
            .unwrap_or_default();
        ProxySettings { http, https, no_proxy }
    }

    /// The proxy a request to `scheme://host` goes through, or `None` when it goes direct. The scheme is `http`
    /// or `https` (case ignored); any other scheme goes direct. A host on the bypass list goes direct.
    pub fn proxy_for(&self, scheme: &str, host: &str) -> Option<&ProxyServer> {
        if self.bypasses(host) {
            return None;
        }
        if scheme.eq_ignore_ascii_case("http") {
            self.http.as_ref()
        } else if scheme.eq_ignore_ascii_case("https") {
            self.https.as_ref()
        } else {
            None
        }
    }

    /// Whether the bypass list sends `host` direct.
    pub fn bypasses(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return true;
        }
        self.no_proxy.iter().any(|entry| entry_matches(entry, &host))
    }
}

/// Splits a `no_proxy` value on commas and spaces, dropping empty entries and lower-casing them.
pub fn parse_no_proxy(value: &str) -> Vec<String> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|e| !e.is_empty())
        .map(|e| e.to_ascii_lowercase())
        .collect()
}

fn entry_matches(entry: &str, host: &str) -> bool {
    if entry == "*" {
        return true;
    }
    let entry = entry.trim_end_matches('.');
    let bare = entry.strip_prefix('.').unwrap_or(entry);
    if bare.is_empty() {
        return false;
    }
    host == bare || host.ends_with(&format!(".{bare}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn parses_proxy_urls_with_and_without_scheme_and_port() {
        assert_eq!(
            ProxyServer::parse("http://Proxy.Lan:3128/"),
            Ok(ProxyServer { host: "proxy.lan".into(), port: 3128 })
        );
        assert_eq!(ProxyServer::parse("10.0.0.2:8080"), Ok(ProxyServer { host: "10.0.0.2".into(), port: 8080 }));
        assert_eq!(ProxyServer::parse("https://secure"), Ok(ProxyServer { host: "secure".into(), port: 80 }));
        assert_eq!(ProxyServer::parse("[fe80::1]:3128"), Ok(ProxyServer { host: "fe80::1".into(), port: 3128 }));
    }

    #[test]
    fn rejects_empty_scheme_and_port_errors() {
        assert_eq!(ProxyServer::parse("  "), Err(ProxyError::Empty));
        assert_eq!(ProxyServer::parse("http://"), Err(ProxyError::Empty));
        assert_eq!(ProxyServer::parse(":3128"), Err(ProxyError::Empty));
        assert_eq!(ProxyServer::parse("socks5://host:1080"), Err(ProxyError::BadScheme("socks5".into())));
        assert_eq!(ProxyServer::parse("host:0"), Err(ProxyError::BadPort("0".into())));
        assert_eq!(ProxyServer::parse("host:70000"), Err(ProxyError::BadPort("70000".into())));
        assert_eq!(ProxyServer::parse("[::1"), Err(ProxyError::BadPort("[::1".into())));
    }

    #[test]
    fn reads_the_environment_with_lower_case_winning() {
        let s = ProxySettings::from_env(env(&[
            ("HTTP_PROXY", "http://upper:1"),
            ("http_proxy", "http://lower:2"),
            ("https_proxy", "http://secure:3"),
            ("NO_PROXY", "local.test"),
        ]));
        assert_eq!(s.http, Some(ProxyServer { host: "lower".into(), port: 2 }));
        assert_eq!(s.https, Some(ProxyServer { host: "secure".into(), port: 3 }));
        assert_eq!(s.no_proxy, vec!["local.test".to_string()]);
    }

    #[test]
    fn a_blank_or_bad_variable_is_not_set() {
        let s = ProxySettings::from_env(env(&[("http_proxy", "   "), ("HTTP_PROXY", "socks5://x:1"), ("https_proxy", "")]));
        assert_eq!(s, ProxySettings::default());
    }

    #[test]
    fn picks_the_proxy_by_scheme_and_sends_other_schemes_direct() {
        let s = ProxySettings::from_env(env(&[("http_proxy", "h:1"), ("https_proxy", "s:2")]));
        assert_eq!(s.proxy_for("http", "example.org").map(|p| p.port), Some(1));
        assert_eq!(s.proxy_for("HTTPS", "example.org").map(|p| p.port), Some(2));
        assert_eq!(s.proxy_for("ftp", "example.org"), None);
    }

    #[test]
    fn bypass_list_matches_exact_hosts_and_subdomains() {
        let s = ProxySettings::from_env(env(&[
            ("http_proxy", "h:1"),
            ("no_proxy", "localhost, .lan example.com"),
        ]));
        assert!(s.bypasses("localhost"));
        assert!(s.bypasses("printer.lan"));
        assert!(s.bypasses("lan"));
        assert!(s.bypasses("example.com"));
        assert!(s.bypasses("A.Example.COM"));
        assert!(!s.bypasses("notexample.com"));
        assert!(!s.bypasses("example.org"));
        assert_eq!(s.proxy_for("http", "printer.lan"), None);
        assert_eq!(s.proxy_for("http", "example.org").map(|p| p.port), Some(1));
    }

    #[test]
    fn star_bypasses_every_host() {
        let s = ProxySettings { http: ProxyServer::parse("h:1").ok(), https: None, no_proxy: parse_no_proxy("*") };
        assert!(s.bypasses("anything.example"));
        assert_eq!(s.proxy_for("http", "anything.example"), None);
    }

    #[test]
    fn trailing_dot_on_host_or_entry_is_ignored() {
        let s = ProxySettings { http: None, https: None, no_proxy: parse_no_proxy("example.com.") };
        assert!(s.bypasses("example.com."));
        assert!(s.bypasses("www.example.com"));
    }

    #[test]
    fn empty_host_goes_direct() {
        let s = ProxySettings { http: ProxyServer::parse("h:1").ok(), https: None, no_proxy: vec![] };
        assert!(s.bypasses(""));
        assert_eq!(s.proxy_for("http", " "), None);
    }
}
