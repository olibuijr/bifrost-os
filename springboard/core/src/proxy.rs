//! System proxy settings: one endpoint per protocol, a bypass list, and the location of an auto-configuration script,
//! plus the resolution of a URL to the ordered list of proxies to try, or to a direct connection.
//!
//! This is a pure model. It reads no settings store, runs no auto-configuration script and opens no socket. The
//! caller fills [`ProxySettings`] from the desktop's network settings and asks [`ProxySettings::choose`] for every
//! request. A script URL is stored and reported, but its result is not evaluated here: a caller that needs script
//! routing has to run the script itself and hand the answer back as endpoints.
use std::fmt;

/// A proxy server: a host name or address, and a non-zero port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyEndpoint {
    host: String,
    port: u16,
}

impl ProxyEndpoint {
    /// `None` when the host is empty, contains a scheme, a path or a space, or the port is zero.
    pub fn new(host: &str, port: u16) -> Option<ProxyEndpoint> {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() || port == 0 || host.contains("://") || host.contains('/') || host.chars().any(char::is_whitespace) {
            return None;
        }
        Some(ProxyEndpoint { host, port })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for ProxyEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// The kind of proxy that carries a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    Http,
    Https,
    Socks,
}

/// One step in the list [`ProxySettings::choose`] returns: a proxy to try, or a direct connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyChoice {
    Direct,
    Proxy { kind: ProxyKind, endpoint: ProxyEndpoint },
}

/// The system proxy configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxySettings {
    pub http: Option<ProxyEndpoint>,
    pub https: Option<ProxyEndpoint>,
    pub socks: Option<ProxyEndpoint>,
    /// Host patterns that skip the proxies: `example.com` (the host and its subdomains), `*.corp.example` (subdomains
    /// only), or an exact host. Compared case-insensitively.
    pub bypass: Vec<String>,
    /// Bypass every host without a dot, such as `intranet`.
    pub bypass_simple_hostnames: bool,
    /// Address of a proxy auto-configuration script. Stored for the caller; not evaluated by this module.
    pub auto_config_url: Option<String>,
}

impl ProxySettings {
    /// Whether `host` matches the bypass list, or is a simple host name while those are bypassed.
    pub fn bypasses(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return true;
        }
        if self.bypass_simple_hostnames && !host.contains('.') {
            return true;
        }
        self.bypass.iter().any(|pattern| {
            let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
            if let Some(suffix) = pattern.strip_prefix("*.") {
                host.ends_with(&format!(".{suffix}"))
            } else {
                host == pattern || host.ends_with(&format!(".{pattern}"))
            }
        })
    }

    /// The proxies to try for a request to `scheme://host`, in order, with the proxy for that scheme first and the
    /// SOCKS proxy after it. An empty result never happens: with no usable proxy, or for a bypassed host, the answer
    /// is `[Direct]`.
    pub fn choose(&self, scheme: &str, host: &str) -> Vec<ProxyChoice> {
        if self.bypasses(host) {
            return vec![ProxyChoice::Direct];
        }
        let mut out = Vec::new();
        let scheme_proxy = match scheme.to_ascii_lowercase().as_str() {
            "http" => self.http.clone().map(|endpoint| (ProxyKind::Http, endpoint)),
            "https" => self.https.clone().map(|endpoint| (ProxyKind::Https, endpoint)),
            _ => None,
        };
        if let Some((kind, endpoint)) = scheme_proxy {
            out.push(ProxyChoice::Proxy { kind, endpoint });
        }
        if let Some(endpoint) = self.socks.clone() {
            out.push(ProxyChoice::Proxy { kind: ProxyKind::Socks, endpoint });
        }
        if out.is_empty() {
            out.push(ProxyChoice::Direct);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(host: &str, port: u16) -> ProxyEndpoint {
        ProxyEndpoint::new(host, port).unwrap()
    }

    fn settings() -> ProxySettings {
        ProxySettings {
            http: Some(endpoint("proxy.example", 3128)),
            https: Some(endpoint("secure.example", 8443)),
            socks: Some(endpoint("socks.example", 1080)),
            bypass: vec!["localhost".into(), "*.corp.example".into()],
            ..Default::default()
        }
    }

    #[test]
    fn an_endpoint_needs_a_host_and_a_port() {
        assert_eq!(ProxyEndpoint::new(" Proxy.Example ", 80).unwrap().host(), "proxy.example");
        assert!(ProxyEndpoint::new("", 80).is_none());
        assert!(ProxyEndpoint::new("proxy.example", 0).is_none());
        assert!(ProxyEndpoint::new("http://proxy.example", 80).is_none());
        assert!(ProxyEndpoint::new("proxy example", 80).is_none());
        assert_eq!(endpoint("10.0.0.2", 3128).to_string(), "10.0.0.2:3128");
    }

    #[test]
    fn a_request_goes_through_the_proxy_for_its_scheme_then_socks() {
        let s = settings();
        assert_eq!(
            s.choose("http", "example.org"),
            vec![
                ProxyChoice::Proxy { kind: ProxyKind::Http, endpoint: endpoint("proxy.example", 3128) },
                ProxyChoice::Proxy { kind: ProxyKind::Socks, endpoint: endpoint("socks.example", 1080) },
            ]
        );
        assert_eq!(
            s.choose("HTTPS", "example.org")[0],
            ProxyChoice::Proxy { kind: ProxyKind::Https, endpoint: endpoint("secure.example", 8443) }
        );
    }

    #[test]
    fn bypassed_hosts_connect_directly() {
        let s = settings();
        assert_eq!(s.choose("https", "localhost"), vec![ProxyChoice::Direct]);
        assert_eq!(s.choose("https", "wiki.corp.example."), vec![ProxyChoice::Direct]);
        assert!(!s.bypasses("corp.example.net"));
        assert!(!s.bypasses("notcorp.example"), "a suffix must start at a dot");
        assert!(!s.bypasses("corp.example"), "the wildcard form does not match the bare domain");
    }

    #[test]
    fn a_plain_domain_pattern_covers_its_subdomains() {
        let s = ProxySettings { bypass: vec!["example.com".into()], ..Default::default() };
        assert!(s.bypasses("a.b.example.com"));
        assert!(s.bypasses("EXAMPLE.com"));
        assert!(!s.bypasses("badexample.com"));
    }

    #[test]
    fn simple_host_names_can_bypass_the_proxy() {
        let mut s = settings();
        assert!(!s.bypasses("intranet"));
        s.bypass_simple_hostnames = true;
        assert!(s.bypasses("intranet"));
        assert!(!s.bypasses("intranet.example.org"));
    }

    #[test]
    fn without_a_usable_proxy_the_answer_is_direct() {
        let none = ProxySettings::default();
        assert_eq!(none.choose("http", "example.org"), vec![ProxyChoice::Direct]);
        let socks_only = ProxySettings { socks: Some(endpoint("socks.example", 1080)), ..Default::default() };
        assert_eq!(
            socks_only.choose("ftp", "example.org"),
            vec![ProxyChoice::Proxy { kind: ProxyKind::Socks, endpoint: endpoint("socks.example", 1080) }]
        );
        let scripted = ProxySettings { auto_config_url: Some("https://config.example/proxy.pac".into()), ..Default::default() };
        assert_eq!(scripted.choose("https", "example.org"), vec![ProxyChoice::Direct]);
    }
}
