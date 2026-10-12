//! System-wide encrypted DNS: the resolver a user saves (DNS over HTTPS or DNS over TLS), the server addresses it
//! uses, and the domains whose queries use it.
//!
//! The model owns no socket and sends no query. A resolver reads a [`DnsConfiguration`], asks
//! [`DnsConfiguration::resolver_for`] which settings apply to a name, and sends the query itself. Domain matching is
//! the usual suffix rule: `example.com` matches `example.com` and every name under it, and never `badexample.com`.
//! Empty match domains mean that no query uses the settings (a rule of this model).

use std::fmt;
use std::net::IpAddr;

/// The transport that carries queries to the DNS server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsProtocol {
    /// DNS over HTTPS: the server is an `https://` URL.
    Https { server_url: String },
    /// DNS over TLS: the TLS name the server presents during the handshake.
    Tls { server_name: String },
}

/// The resolver settings: one protocol, the server addresses (tried in order) and the domains that use them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsSettings {
    pub protocol: DnsProtocol,
    pub servers: Vec<IpAddr>,
    pub match_domains: Vec<String>,
}

/// Why a DNS setting is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsError {
    NoServers,
    DuplicateServer(IpAddr),
    BadServerUrl(String),
    EmptyServerName,
    BadMatchDomain(String),
    NotConfigured,
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DnsError::NoServers => write!(f, "at least one DNS server address is needed"),
            DnsError::DuplicateServer(ip) => write!(f, "the DNS server {ip} is listed twice"),
            DnsError::BadServerUrl(url) => write!(f, "the DNS server URL {url:?} is not an https:// URL"),
            DnsError::EmptyServerName => write!(f, "the TLS name of the DNS server is empty"),
            DnsError::BadMatchDomain(d) => write!(f, "{d:?} is not a domain name"),
            DnsError::NotConfigured => write!(f, "the DNS settings have no servers yet"),
        }
    }
}

/// Lower case, no surrounding space, no trailing dot.
pub fn normalise_domain(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// True when `host` is `domain` or a name under it. An empty domain or host matches nothing.
pub fn domain_matches(host: &str, domain: &str) -> bool {
    let host = normalise_domain(host);
    let domain = normalise_domain(domain);
    if host.is_empty() || domain.is_empty() {
        return false;
    }
    host == domain || host.ends_with(format!(".{domain}").as_str())
}

/// True when `name` is a dotted name whose labels are letters, digits or hyphens (no label is empty, and no label
/// starts or ends with a hyphen).
pub fn valid_domain(name: &str) -> bool {
    let n = normalise_domain(name);
    !n.is_empty()
        && n.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

impl DnsSettings {
    /// Builds settings without checking them. Call [`DnsSettings::validate`] before use.
    pub fn new(protocol: DnsProtocol, servers: Vec<IpAddr>) -> Self {
        DnsSettings { protocol, servers, match_domains: Vec::new() }
    }

    /// Checks the server list, the protocol's server value and every match domain.
    pub fn validate(&self) -> Result<(), DnsError> {
        if self.servers.is_empty() {
            return Err(DnsError::NoServers);
        }
        for (i, ip) in self.servers.iter().enumerate() {
            if self.servers[..i].contains(ip) {
                return Err(DnsError::DuplicateServer(*ip));
            }
        }
        match &self.protocol {
            DnsProtocol::Https { server_url } => {
                let host = server_url.strip_prefix("https://").map(|rest| rest.split('/').next().unwrap_or(""));
                let ok = host.is_some_and(|h| !h.is_empty()) && !server_url.chars().any(char::is_whitespace);
                if !ok {
                    return Err(DnsError::BadServerUrl(server_url.clone()));
                }
            }
            DnsProtocol::Tls { server_name } => {
                if server_name.trim().is_empty() {
                    return Err(DnsError::EmptyServerName);
                }
            }
        }
        for d in &self.match_domains {
            if !valid_domain(d) {
                return Err(DnsError::BadMatchDomain(d.clone()));
            }
        }
        Ok(())
    }
}

/// The saved DNS configuration: whether it is on, its display name and its settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsConfiguration {
    pub enabled: bool,
    pub description: String,
    settings: Option<DnsSettings>,
}

impl DnsConfiguration {
    /// A configuration that is off and has no settings.
    pub fn new(description: &str) -> Self {
        DnsConfiguration { enabled: false, description: description.to_string(), settings: None }
    }

    /// The saved settings, if any.
    pub fn settings(&self) -> Option<&DnsSettings> {
        self.settings.as_ref()
    }

    /// Replaces the settings after they pass validation. A refused value leaves the saved settings unchanged.
    pub fn set_settings(&mut self, settings: DnsSettings) -> Result<(), DnsError> {
        settings.validate()?;
        self.settings = Some(settings);
        Ok(())
    }

    /// Turns the configuration on. It needs saved settings.
    pub fn enable(&mut self) -> Result<(), DnsError> {
        if self.settings.is_none() {
            return Err(DnsError::NotConfigured);
        }
        self.enabled = true;
        Ok(())
    }

    /// The settings a query for `host` uses: the configuration is on and a match domain covers the name.
    pub fn resolver_for(&self, host: &str) -> Option<&DnsSettings> {
        if !self.enabled {
            return None;
        }
        let s = self.settings.as_ref()?;
        if s.match_domains.iter().any(|d| domain_matches(host, d)) {
            Some(s)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn https(url: &str) -> DnsSettings {
        DnsSettings::new(DnsProtocol::Https { server_url: url.to_string() }, vec![ip("192.0.2.53")])
    }

    #[test]
    fn a_domain_matches_itself_and_its_subdomains_only() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(domain_matches("WWW.Example.com.", "example.com"));
        assert!(domain_matches("a.b.example.com", "example.com."));
        assert!(!domain_matches("badexample.com", "example.com"));
        assert!(!domain_matches("example.com.evil", "example.com"));
        assert!(!domain_matches("example.com", ""));
        assert!(!domain_matches("", "example.com"));
    }

    #[test]
    fn a_https_server_needs_an_https_url_with_a_host() {
        assert!(https("https://dns.example/dns-query").validate().is_ok());
        assert_eq!(
            https("http://dns.example/dns-query").validate(),
            Err(DnsError::BadServerUrl("http://dns.example/dns-query".to_string()))
        );
        assert!(https("https:///dns-query").validate().is_err());
        assert!(https("https://").validate().is_err());
        assert!(https("https://dns .example").validate().is_err());
    }

    #[test]
    fn the_server_list_and_the_match_domains_are_checked() {
        let none = DnsSettings::new(DnsProtocol::Tls { server_name: "dns.example".into() }, vec![]);
        assert_eq!(none.validate(), Err(DnsError::NoServers));
        let twice = DnsSettings::new(
            DnsProtocol::Tls { server_name: "dns.example".into() },
            vec![ip("192.0.2.1"), ip("192.0.2.1")],
        );
        assert_eq!(twice.validate(), Err(DnsError::DuplicateServer(ip("192.0.2.1"))));
        let blank = DnsSettings::new(DnsProtocol::Tls { server_name: "  ".into() }, vec![ip("192.0.2.1")]);
        assert_eq!(blank.validate(), Err(DnsError::EmptyServerName));
        let mut bad = https("https://dns.example/q");
        bad.match_domains = vec!["corp..example".to_string()];
        assert_eq!(bad.validate(), Err(DnsError::BadMatchDomain("corp..example".to_string())));
    }

    #[test]
    fn a_disabled_configuration_answers_no_name() {
        let mut c = DnsConfiguration::new("Office DNS");
        let mut s = https("https://dns.example/q");
        s.match_domains = vec!["corp.example".to_string()];
        c.set_settings(s).unwrap();
        assert!(c.resolver_for("mail.corp.example").is_none(), "off means no settings apply");
        c.enable().unwrap();
        assert!(c.resolver_for("mail.corp.example").is_some());
        assert!(c.resolver_for("example.com").is_none(), "a name outside the match domains uses the system resolver");
    }

    #[test]
    fn a_refused_setting_keeps_the_saved_one_and_enabling_needs_settings() {
        let mut c = DnsConfiguration::new("Office DNS");
        assert_eq!(c.enable(), Err(DnsError::NotConfigured));
        assert!(!c.enabled);
        let mut good = https("https://dns.example/q");
        good.match_domains = vec!["corp.example".to_string()];
        c.set_settings(good.clone()).unwrap();
        assert!(c.set_settings(https("http://bad.example/q")).is_err());
        assert_eq!(c.settings(), Some(&good));
        assert_eq!(DnsError::NoServers.to_string(), "at least one DNS server address is needed");
    }
}
