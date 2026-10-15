//! HTTP exchange logging for apps: one log line per request with its method, a redacted URL, the outcome, the body
//! size and the duration, at the level the outcome warrants. The URL is redacted before anything is written: the values
//! of credential-like query parameters and a password in the authority are replaced, and fragments (never sent to a
//! server) are dropped. Headers and bodies are not logged.
//!
//! ```ignore
//! let exchange = Exchange::response("GET", url, 200, bytes, elapsed);
//! exchange.record(&logger);
//! ```
//!
//! Packet traces and an on-screen network inspector are not part of this module.

use crate::log::{Level, Logger};
use std::time::Duration;

/// What replaces a redacted value in a URL.
pub const REDACTED: &str = "<redacted>";

/// Query parameter names whose values are never logged, compared without case.
const SENSITIVE_PARAMETERS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "auth",
    "code",
    "key",
    "password",
    "pass",
    "refresh_token",
    "secret",
    "session",
    "sig",
    "signature",
    "token",
];

/// A URL as it may be written to a log: credential-like query values and an authority password are replaced, and the
/// fragment is dropped.
pub fn redact_url(url: &str) -> String {
    let without_fragment = url.split_once('#').map_or(url, |(before, _)| before);
    let (base, query) = match without_fragment.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (without_fragment, None),
    };
    let mut out = redact_authority(base);
    if let Some(query) = query {
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((name, _)) if is_sensitive(name) => format!("{name}={REDACTED}"),
                _ => pair.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    out
}

fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_PARAMETERS.contains(&lower.as_str())
}

/// Replaces the password of `scheme://user:password@host/...`. A URL without one is returned unchanged.
fn redact_authority(base: &str) -> String {
    let Some(scheme_end) = base.find("://") else { return base.to_string() };
    let (scheme, after) = base.split_at(scheme_end + 3);
    let authority_end = after.find('/').unwrap_or(after.len());
    let (authority, path) = after.split_at(authority_end);
    match authority.rsplit_once('@') {
        Some((userinfo, host)) => match userinfo.split_once(':') {
            Some((user, _)) => format!("{scheme}{user}:{REDACTED}@{host}{path}"),
            None => base.to_string(),
        },
        None => base.to_string(),
    }
}

/// How an exchange ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A response arrived with this status code.
    Response { status: u16 },
    /// No response: the reason is the error text, such as a timeout or a refused connection.
    Failed { reason: String },
}

/// One finished request: what was asked, how it ended, the response body size and the time it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub method: String,
    pub url: String,
    pub outcome: Outcome,
    pub bytes: u64,
    pub elapsed: Duration,
}

impl Exchange {
    /// An exchange that got a response.
    pub fn response(method: &str, url: &str, status: u16, bytes: u64, elapsed: Duration) -> Exchange {
        Exchange { method: method.to_string(), url: url.to_string(), outcome: Outcome::Response { status }, bytes, elapsed }
    }

    /// An exchange that got no response.
    pub fn failed(method: &str, url: &str, reason: &str, elapsed: Duration) -> Exchange {
        Exchange {
            method: method.to_string(),
            url: url.to_string(),
            outcome: Outcome::Failed { reason: reason.to_string() },
            bytes: 0,
            elapsed,
        }
    }

    /// The level the outcome is logged at: successes and redirects at debug, client errors at notice, server errors
    /// and failed requests at error.
    pub fn level(&self) -> Level {
        match self.outcome {
            Outcome::Response { status } if status >= 500 => Level::Error,
            Outcome::Response { status } if status >= 400 => Level::Notice,
            Outcome::Response { .. } => Level::Debug,
            Outcome::Failed { .. } => Level::Error,
        }
    }

    /// The one-line summary, with the URL redacted, for example `GET https://example.test/feed 200 1.5 KiB in 42 ms`.
    pub fn summary(&self) -> String {
        let url = redact_url(&self.url);
        let ms = self.elapsed.as_millis();
        match &self.outcome {
            Outcome::Response { status } => {
                format!("{} {} {} {} in {} ms", self.method, url, status, body_size(self.bytes), ms)
            }
            Outcome::Failed { reason } => format!("{} {} failed after {} ms: {}", self.method, url, ms, reason),
        }
    }

    /// Writes the summary to a logger at the level of the outcome.
    pub fn record(&self, logger: &Logger) {
        logger.log(self.level(), &self.summary());
    }
}

/// A body size for a log line: bytes below a kibibyte, then kibibytes and mebibytes with one decimal.
pub fn body_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}
