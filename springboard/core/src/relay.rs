//! Network relays: the saved configuration for one or two relay servers that carry connections, the domains that use
//! them, and the rule that picks the relays for a host.
//!
//! The model opens no connection. A relay client reads a [`RelayConfiguration`] and asks
//! [`RelayConfiguration::relays_for`] which relays a host goes through; the client makes the connections itself. With
//! two relays, traffic passes through both in the order they are listed. Each relay is reached over HTTP/2, over
//! HTTP/3, or both, and needs at least one `https://` URL. Excluded domains win over match domains (a rule of this
//! model), and domains use the suffix rule from [`crate::dns_settings`].

use crate::dns_settings::{domain_matches, valid_domain};
use std::fmt;

/// The most relays one configuration may chain.
pub const MAX_RELAYS: usize = 2;

/// One relay server: the URL for each protocol it is reached with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayServer {
    pub http2_url: Option<String>,
    pub http3_url: Option<String>,
}

/// Why a relay configuration is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayError {
    NoRelays,
    TooManyRelays(usize),
    /// The relay at this index has no URL.
    NoUrl(usize),
    BadUrl(String),
    BadDomain(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::NoRelays => write!(f, "a relay configuration needs at least one relay"),
            RelayError::TooManyRelays(n) => write!(f, "{n} relays are configured; at most {MAX_RELAYS} are allowed"),
            RelayError::NoUrl(i) => write!(f, "relay {} has no HTTP/2 or HTTP/3 URL", i + 1),
            RelayError::BadUrl(url) => write!(f, "the relay URL {url:?} is not an https:// URL"),
            RelayError::BadDomain(d) => write!(f, "{d:?} is not a domain name"),
        }
    }
}

/// The saved relay configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayConfiguration {
    pub enabled: bool,
    pub description: String,
    pub relays: Vec<RelayServer>,
    pub match_domains: Vec<String>,
    pub excluded_domains: Vec<String>,
}

/// True when `url` is `https://` followed by a host, with no white space anywhere.
fn is_https_url(url: &str) -> bool {
    match url.strip_prefix("https://") {
        Some(rest) => !rest.split('/').next().unwrap_or("").is_empty() && !url.chars().any(char::is_whitespace),
        None => false,
    }
}

impl RelayConfiguration {
    /// A configuration that is off, with no relays and no domains.
    pub fn new(description: &str) -> Self {
        RelayConfiguration { description: description.to_string(), ..Default::default() }
    }

    /// Checks the relay count, each relay's URLs and every domain.
    pub fn validate(&self) -> Result<(), RelayError> {
        if self.relays.is_empty() {
            return Err(RelayError::NoRelays);
        }
        if self.relays.len() > MAX_RELAYS {
            return Err(RelayError::TooManyRelays(self.relays.len()));
        }
        for (i, relay) in self.relays.iter().enumerate() {
            let urls = [relay.http2_url.as_deref(), relay.http3_url.as_deref()];
            if urls.iter().all(|u| u.is_none()) {
                return Err(RelayError::NoUrl(i));
            }
            for url in urls.iter().flatten().copied() {
                if !is_https_url(url) {
                    return Err(RelayError::BadUrl(url.to_string()));
                }
            }
        }
        for d in self.match_domains.iter().chain(self.excluded_domains.iter()) {
            if !valid_domain(d) {
                return Err(RelayError::BadDomain(d.clone()));
            }
        }
        Ok(())
    }

    /// Turns the configuration on or off. Turning it on runs [`RelayConfiguration::validate`] first.
    pub fn set_enabled(&mut self, on: bool) -> Result<(), RelayError> {
        if on {
            self.validate()?;
        }
        self.enabled = on;
        Ok(())
    }

    /// The relays a connection to `host` goes through, in order. None when the configuration is off, the host is
    /// excluded, or no match domain covers it.
    pub fn relays_for(&self, host: &str) -> Option<&[RelayServer]> {
        if !self.enabled {
            return None;
        }
        if self.excluded_domains.iter().any(|d| domain_matches(host, d)) {
            return None;
        }
        if self.match_domains.iter().any(|d| domain_matches(host, d)) {
            Some(self.relays.as_slice())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(http2: &str) -> RelayServer {
        RelayServer { http2_url: Some(format!("https://{http2}/relay")), http3_url: None }
    }

    fn working() -> RelayConfiguration {
        let mut c = RelayConfiguration::new("Work relay");
        c.relays = vec![relay("first.example"), relay("second.example")];
        c.match_domains = vec!["example.com".to_string()];
        c
    }

    #[test]
    fn the_relay_count_and_each_relay_url_are_checked() {
        let mut c = RelayConfiguration::new("x");
        assert_eq!(c.validate(), Err(RelayError::NoRelays));
        c.relays = vec![relay("a.example"), relay("b.example"), relay("c.example")];
        assert_eq!(c.validate(), Err(RelayError::TooManyRelays(3)));
        c.relays = vec![RelayServer::default()];
        assert_eq!(c.validate(), Err(RelayError::NoUrl(0)));
        c.relays = vec![RelayServer { http2_url: None, http3_url: Some("http://plain.example/r".into()) }];
        assert_eq!(c.validate(), Err(RelayError::BadUrl("http://plain.example/r".to_string())));
        c.relays = vec![RelayServer { http2_url: None, http3_url: Some("https://h3.example/r".into()) }];
        assert!(c.validate().is_ok(), "an HTTP/3 URL alone is a valid relay");
    }

    #[test]
    fn a_host_under_a_match_domain_goes_through_both_relays_in_order() {
        let mut c = working();
        assert!(c.relays_for("www.example.com").is_none(), "off means no relay");
        c.set_enabled(true).unwrap();
        let used = c.relays_for("www.example.com").unwrap();
        assert_eq!(used.len(), 2);
        assert_eq!(used[0].http2_url.as_deref(), Some("https://first.example/relay"));
        assert_eq!(used[1].http2_url.as_deref(), Some("https://second.example/relay"));
        assert!(c.relays_for("other.org").is_none());
        assert!(c.relays_for("badexample.com").is_none());
    }

    #[test]
    fn an_excluded_domain_wins_over_a_match_domain() {
        let mut c = working();
        c.excluded_domains = vec!["bank.example.com".to_string()];
        c.set_enabled(true).unwrap();
        assert!(c.relays_for("app.bank.example.com").is_none());
        assert!(c.relays_for("shop.example.com").is_some());
    }

    #[test]
    fn turning_on_an_invalid_configuration_is_refused() {
        let mut c = RelayConfiguration::new("bad");
        c.relays = vec![relay("a.example")];
        c.match_domains = vec!["a..example".to_string()];
        assert_eq!(c.set_enabled(true), Err(RelayError::BadDomain("a..example".to_string())));
        assert!(!c.enabled, "a refused switch leaves the configuration off");
        assert_eq!(
            RelayError::NoUrl(1).to_string(),
            "relay 2 has no HTTP/2 or HTTP/3 URL"
        );
    }
}
