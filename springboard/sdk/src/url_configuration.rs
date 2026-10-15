//! URL session configuration (`URLSessionConfiguration`): the defaults a session gives the requests it runs. The
//! configuration caps each request's timeout, adds the session's headers to requests that do not set them, decides how
//! a request meets the cache, and says when a whole transfer has run out of time. Wiring the values into the transport
//! (connection pooling, HTTP/2, cellular policy) belongs to the transport; this module holds the rules.
//!
//! ```ignore
//! let mut config = URLSessionConfiguration::default();
//! config.set_header("Accept-Language", "is");
//! config.timeout_interval_for_request = Duration::from_secs(15);
//! config.prepare(&mut request);
//! ```

use crate::url::URLRequest;
use std::time::Duration;

/// How a request uses the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePolicy {
    /// Use a cached response while it is fresh, otherwise load it.
    UseProtocolCachePolicy,
    /// Always load from the network.
    ReloadIgnoringLocalCache,
    /// Use a cached response whether or not it is fresh, and load only when nothing is cached.
    ReturnCacheDataElseLoad,
    /// Use a cached response when there is one, and fail rather than load when there is none (offline mode).
    ReturnCacheDataDontLoad,
}

/// What a request does with the cache under a policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheAction {
    /// Ask the network for the response.
    Load,
    /// Answer from the cached response.
    UseCache,
    /// Fail the request without contacting the network.
    Fail,
}

impl CachePolicy {
    /// The action for a request given the cache entry: `None` when nothing is cached, `Some(fresh)` when an entry is
    /// stored and says whether it is still fresh.
    pub fn action(self, entry: Option<bool>) -> CacheAction {
        match (self, entry) {
            (CachePolicy::ReloadIgnoringLocalCache, _) => CacheAction::Load,
            (CachePolicy::UseProtocolCachePolicy, Some(true)) => CacheAction::UseCache,
            (CachePolicy::UseProtocolCachePolicy, _) => CacheAction::Load,
            (CachePolicy::ReturnCacheDataElseLoad, Some(_)) => CacheAction::UseCache,
            (CachePolicy::ReturnCacheDataElseLoad, None) => CacheAction::Load,
            (CachePolicy::ReturnCacheDataDontLoad, Some(_)) => CacheAction::UseCache,
            (CachePolicy::ReturnCacheDataDontLoad, None) => CacheAction::Fail,
        }
    }
}

/// The defaults of a session and the rules its requests follow.
#[derive(Clone, Debug, PartialEq)]
pub struct URLSessionConfiguration {
    /// Longest wait for one request.
    pub timeout_interval_for_request: Duration,
    /// Longest time for a whole transfer, retries included.
    pub timeout_interval_for_resource: Duration,
    /// Connections a session keeps open to one host.
    pub http_maximum_connections_per_host: usize,
    /// Headers added to every request that does not set them.
    pub http_additional_headers: Vec<(String, String)>,
    /// Whether requests may use the cellular interface.
    pub allows_cellular_access: bool,
    /// Whether a request waits for a connection rather than failing at once.
    pub waits_for_connectivity: bool,
    /// The cache policy of requests that do not set their own.
    pub request_cache_policy: CachePolicy,
}

impl Default for URLSessionConfiguration {
    /// The usual values: 60 s per request, 7 days per resource, six connections per host, no extra headers, cellular
    /// access allowed, no waiting for connectivity, and the protocol cache policy.
    fn default() -> Self {
        URLSessionConfiguration {
            timeout_interval_for_request: Duration::from_secs(60),
            timeout_interval_for_resource: Duration::from_secs(7 * 24 * 60 * 60),
            http_maximum_connections_per_host: 6,
            http_additional_headers: Vec::new(),
            allows_cellular_access: true,
            waits_for_connectivity: false,
            request_cache_policy: CachePolicy::UseProtocolCachePolicy,
        }
    }
}

impl URLSessionConfiguration {
    /// Sets a session header, replacing one of the same name (compared without case).
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.http_additional_headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.http_additional_headers.push((name.to_string(), value.to_string()));
    }

    /// The number of connections a session may open to one host: the configured value, and at least one.
    pub fn connection_limit(&self) -> usize {
        self.http_maximum_connections_per_host.max(1)
    }

    /// Whether a transfer that has run for `elapsed` has used up its resource timeout.
    pub fn resource_expired(&self, elapsed: Duration) -> bool {
        elapsed >= self.timeout_interval_for_resource
    }

    /// Applies the session to a request before it runs: the session headers the request does not set are added, and
    /// the request's timeout is capped at the session's request timeout.
    pub fn prepare(&self, request: &mut URLRequest) {
        for (name, value) in &self.http_additional_headers {
            if !request.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
                request.headers.push((name.clone(), value.clone()));
            }
        }
        request.timeout = request.timeout.min(self.timeout_interval_for_request);
    }
}
