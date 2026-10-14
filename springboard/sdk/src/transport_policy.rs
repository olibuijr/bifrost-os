//! Transport policy for network links: which requests may use plain http, and whether a redirect may change the scheme.
//!
//! The default policy allows only https. An app can except hosts from that rule, as an exception list in its manifest
//! does: an excepted host may use http, and an exception can cover the host's subdomains as well. The policy reads no
//! manifest and opens no connection. The URL client refuses https-to-http redirects on its own; an app or the shell
//! that follows redirects itself checks each hop with [`TransportPolicy::check_redirect`].

use crate::url::parse_url;

/// Why a request or a redirect is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// The address is not a valid http or https URL.
    BadURL(String),
    /// The address uses plain http and its host is not excepted.
    InsecureRequest(String),
    /// A redirect would move an https request to plain http for a host that is not excepted.
    RedirectDowngrade { from: String, to: String },
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::BadURL(url) => write!(f, "Bad URL: {url}"),
            PolicyError::InsecureRequest(url) => write!(f, "Plain http is not allowed for {url}"),
            PolicyError::RedirectDowngrade { from, to } => write!(f, "Redirect from {from} to plain http {to} is refused"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Exception {
    host: String,
    include_subdomains: bool,
}

/// The rules for plain http. The default policy has no exceptions, so only https passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransportPolicy {
    exceptions: Vec<Exception>,
}

impl TransportPolicy {
    /// A policy that allows only https.
    pub fn secure_only() -> TransportPolicy {
        TransportPolicy::default()
    }

    /// Lets plain http reach `host`. With `include_subdomains`, it also lets plain http reach hosts under `host`. Host
    /// names are compared without regard to case.
    pub fn allow_insecure_host(mut self, host: &str, include_subdomains: bool) -> TransportPolicy {
        self.exceptions.push(Exception {
            host: host.to_ascii_lowercase(),
            include_subdomains,
        });
        self
    }

    /// Checks a request before it is sent: https always passes, plain http passes only for an excepted host.
    pub fn check_request(&self, url: &str) -> Result<(), PolicyError> {
        let target = parse(url)?;
        if target.tls || self.allows_insecure(&target.host) {
            Ok(())
        } else {
            Err(PolicyError::InsecureRequest(url.to_string()))
        }
    }

    /// Checks a redirect to the absolute address `to`, sent in answer to a request for `from`. A redirect that keeps
    /// https, or that goes from http to any scheme, passes. A redirect from https to plain http passes only for an
    /// excepted host.
    pub fn check_redirect(&self, from: &str, to: &str) -> Result<(), PolicyError> {
        let source = parse(from)?;
        let target = parse(to)?;
        if source.tls && !target.tls && !self.allows_insecure(&target.host) {
            return Err(PolicyError::RedirectDowngrade {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        Ok(())
    }

    fn allows_insecure(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.exceptions.iter().any(|e| {
            host == e.host || (e.include_subdomains && host.ends_with(&format!(".{}", e.host)))
        })
    }
}

/// The scheme and host of a URL, or a bad-URL error for an address the URL client cannot parse.
struct Target {
    tls: bool,
    host: String,
}

fn parse(url: &str) -> Result<Target, PolicyError> {
    parse_url(url)
        .map(|parsed| Target {
            tls: parsed.tls,
            host: parsed.host,
        })
        .map_err(|_| PolicyError::BadURL(url.to_string()))
}
