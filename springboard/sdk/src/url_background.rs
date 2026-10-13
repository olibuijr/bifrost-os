//! `URLSessionConfiguration` (default, ephemeral, background), a configured session that wires the cookie jar, the
//! credential storage, the disk `URLCache` and authentication challenges into requests, `URLSessionTaskMetrics`, and
//! background sessions whose queued downloads survive a rebuild of the session.
//!
//! `ConfiguredSession::data` is blocking (call it from a background thread). Per request it: applies the cache policy,
//! attaches cookies, sends, stores `Set-Cookie`, and on a 401 with `WWW-Authenticate: Basic` asks the delegate
//! (`urlSession(_:didReceive:completionHandler:)`) and retries once with the chosen credential. Digest is not answered
//! (the challenge goes to the delegate and the 401 is returned unless it cancels or supplies nothing).
//! A background session keeps its queue in `<dir>/<identifier>.queue`; building a session with the same identifier
//! picks the unfinished downloads up again, and `run_pending` finishes them.

use crate::http_auth::{parse_www_authenticate, AuthChallenge, AuthDisposition, URLCredentialStorage, URLProtectionSpace};
use crate::http_cookies::HTTPCookieStorage;
use crate::url::{HTTPURLResponse, URLError, URLRequest, URLSession};
use crate::url_cache::{CacheDecision, CachePolicy, URLCache};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The three kinds of configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationKind {
    Default,
    Ephemeral,
    Background(String),
}

/// `URLSessionConfiguration`.
#[derive(Clone)]
pub struct URLSessionConfiguration {
    pub kind: ConfigurationKind,
    pub request_cache_policy: CachePolicy,
    pub timeout_interval_for_request: Duration,
    pub http_should_set_cookies: bool,
    pub http_cookie_storage: Option<Arc<Mutex<HTTPCookieStorage>>>,
    pub url_credential_storage: Option<Arc<Mutex<URLCredentialStorage>>>,
    pub url_cache: Option<Arc<URLCache>>,
}

impl URLSessionConfiguration {
    /// `.default`: cookies and credentials on, cache `cache`.
    pub fn default_configuration(cache: Option<Arc<URLCache>>) -> Self {
        URLSessionConfiguration {
            kind: ConfigurationKind::Default,
            request_cache_policy: CachePolicy::UseProtocolCachePolicy,
            timeout_interval_for_request: Duration::from_secs(60),
            http_should_set_cookies: true,
            http_cookie_storage: Some(Arc::new(Mutex::new(HTTPCookieStorage::new()))),
            url_credential_storage: Some(Arc::new(Mutex::new(URLCredentialStorage::new()))),
            url_cache: cache,
        }
    }
    /// `.ephemeral`: nothing is cached; cookies and credentials live only in this configuration.
    pub fn ephemeral() -> Self {
        URLSessionConfiguration { kind: ConfigurationKind::Ephemeral, ..Self::default_configuration(None) }
    }
    /// `.background(withIdentifier:)`.
    pub fn background(identifier: &str) -> Self {
        URLSessionConfiguration { kind: ConfigurationKind::Background(identifier.to_string()), ..Self::default_configuration(None) }
    }
    /// `identifier`: set for background configurations only.
    pub fn identifier(&self) -> Option<&str> {
        match &self.kind {
            ConfigurationKind::Background(id) => Some(id),
            _ => None,
        }
    }
}

/// `URLSessionTaskDelegate` (the part that answers challenges).
pub trait URLSessionTaskDelegate: Send + Sync {
    /// `urlSession(_:task:didReceive:completionHandler:)`.
    fn did_receive(&self, challenge: &AuthChallenge) -> AuthDisposition;
}

/// `URLSessionTaskMetrics`, one transaction per request sent.
#[derive(Clone, Debug, PartialEq)]
pub struct URLSessionTaskMetrics {
    pub transactions: Vec<TransactionMetrics>,
    pub task_interval: Duration,
}

/// `URLSessionTaskTransactionMetrics`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransactionMetrics {
    pub url: String,
    pub status_code: Option<u16>,
    pub duration: Duration,
    pub response_body_bytes: usize,
    pub resource_fetch_type: FetchType,
}

/// `URLSessionTaskMetrics.ResourceFetchType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchType {
    NetworkLoad,
    LocalCache,
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A session built from a configuration.
pub struct ConfiguredSession {
    pub configuration: URLSessionConfiguration,
    session: URLSession,
    delegate: Option<Arc<dyn URLSessionTaskDelegate>>,
    last_metrics: Mutex<Option<URLSessionTaskMetrics>>,
}

impl ConfiguredSession {
    /// `URLSession(configuration:delegate:)` over a transport session (`URLSession::shared()` or a test one).
    pub fn new(configuration: URLSessionConfiguration, session: URLSession, delegate: Option<Arc<dyn URLSessionTaskDelegate>>) -> Self {
        ConfiguredSession { configuration, session, delegate, last_metrics: Mutex::new(None) }
    }

    /// `URLSessionTask.metrics` of the latest `data` call.
    pub fn last_metrics(&self) -> Option<URLSessionTaskMetrics> {
        self.last_metrics.lock().unwrap().clone()
    }

    fn send(&self, req: &URLRequest, now: u64, txs: &mut Vec<TransactionMetrics>) -> Result<(HTTPURLResponse, Vec<u8>), URLError> {
        let mut req = req.clone();
        if req.timeout == Duration::from_secs(60) {
            req.timeout = self.configuration.timeout_interval_for_request;
        }
        if self.configuration.http_should_set_cookies {
            if let Some(jar) = &self.configuration.http_cookie_storage {
                crate::url::attach_cookies(&mut req, &jar.lock().unwrap(), now);
            }
        }
        let started = Instant::now();
        let cancel = AtomicBool::new(false);
        let mut body = Vec::new();
        let resp = self.session.bytes(&req, &cancel, &mut |_| {}, &mut |c| {
            body.extend_from_slice(c);
            true
        })?;
        txs.push(TransactionMetrics {
            url: req.url.clone(),
            status_code: Some(resp.status_code),
            duration: started.elapsed(),
            response_body_bytes: body.len(),
            resource_fetch_type: FetchType::NetworkLoad,
        });
        if self.configuration.http_should_set_cookies {
            if let Some(jar) = &self.configuration.http_cookie_storage {
                crate::url::store_cookies(&resp, &mut jar.lock().unwrap(), now);
            }
        }
        Ok((resp, body))
    }

    /// `dataTask`, blocking: cache policy, cookies, one authentication retry, cache store. `now` is Unix seconds.
    pub fn data_at(&self, request: &URLRequest, now: u64) -> Result<(HTTPURLResponse, Vec<u8>), URLError> {
        let begun = Instant::now();
        let mut txs = Vec::new();
        let result = self.data_inner(request, now, &mut txs);
        *self.last_metrics.lock().unwrap() = Some(URLSessionTaskMetrics { transactions: txs, task_interval: begun.elapsed() });
        result
    }

    /// `data_at` with the system clock.
    pub fn data(&self, request: &URLRequest) -> Result<(HTTPURLResponse, Vec<u8>), URLError> {
        self.data_at(request, unix_now())
    }

    fn data_inner(&self, request: &URLRequest, now: u64, txs: &mut Vec<TransactionMetrics>) -> Result<(HTTPURLResponse, Vec<u8>), URLError> {
        let mut req = request.clone();
        let cache = self.configuration.url_cache.as_ref().filter(|_| req.method == "GET");
        let mut revalidating = false;
        if let Some(cache) = cache {
            match cache.decide(&req.url, self.configuration.request_cache_policy, now) {
                CacheDecision::UseCache => {
                    let c = cache.cached_response(&req.url).ok_or(URLError::Cancelled)?;
                    txs.push(TransactionMetrics {
                        url: req.url.clone(),
                        status_code: Some(c.response.status_code),
                        duration: Duration::ZERO,
                        response_body_bytes: c.data.len(),
                        resource_fetch_type: FetchType::LocalCache,
                    });
                    return Ok((c.response, c.data));
                }
                CacheDecision::Fail => return Err(URLError::CannotConnect("not in the cache".into())),
                CacheDecision::Revalidate(tag) => {
                    req.set_header("If-None-Match", &tag);
                    revalidating = true;
                }
                CacheDecision::Load => {}
            }
        }
        let (mut resp, mut body) = self.send(&req, now, txs)?;
        if resp.status_code == 401 {
            if let Some(retry) = self.answer_challenge(&req, &resp) {
                (resp, body) = self.send(&retry, now, txs)?;
            }
        }
        if let Some(cache) = cache {
            if resp.status_code == 304 && revalidating {
                if let Some(c) = cache.renew(&req.url, &resp, now) {
                    return Ok((c.response, c.data));
                }
            } else if resp.status_code == 200 {
                cache.store_cached_response(&resp, &body, now);
            }
        }
        Ok((resp, body))
    }

    /// The request to retry after a Basic challenge, or `None` when the delegate declines or the scheme is not Basic.
    fn answer_challenge(&self, req: &URLRequest, resp: &HTTPURLResponse) -> Option<URLRequest> {
        let challenge = parse_www_authenticate(resp.value("WWW-Authenticate")?)?;
        let space = URLProtectionSpace::for_challenge(&req.url, &challenge)?;
        let storage = self.configuration.url_credential_storage.as_ref()?;
        let ch = AuthChallenge::new(space.clone());
        let disposition = match &self.delegate {
            Some(d) => d.did_receive(&ch),
            None => ch.disposition(&storage.lock().unwrap()),
        };
        match disposition {
            AuthDisposition::UseCredential(cred) if challenge.scheme == "basic" => {
                if cred.persistence != crate::http_auth::CredentialPersistence::None {
                    storage.lock().unwrap().set_credential(cred.clone(), space);
                }
                let mut retry = req.clone();
                cred.apply_to(&mut retry);
                Some(retry)
            }
            _ => None,
        }
    }
}

/// The queue of a background session: downloads that are not finished, kept on disk.
pub struct BackgroundSession {
    identifier: String,
    path: PathBuf,
    pending: Vec<(String, PathBuf)>,
}

impl BackgroundSession {
    /// Opens (or rebuilds) the background session `identifier` with its queue under `dir`.
    pub fn open(identifier: &str, dir: &Path) -> Self {
        let path = dir.join(format!("{identifier}.queue"));
        let pending = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(u, d)| (u.to_string(), PathBuf::from(d))))
            .collect();
        BackgroundSession { identifier: identifier.to_string(), path, pending }
    }
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
    /// The downloads still to run, as `(url, destination)`.
    pub fn pending(&self) -> &[(String, PathBuf)] {
        &self.pending
    }
    fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&self.path)?;
        for (u, d) in &self.pending {
            writeln!(f, "{u}\t{}", d.display())?;
        }
        Ok(())
    }
    /// `downloadTask(with:)` on a background session: queued on disk before anything runs.
    pub fn enqueue_download(&mut self, url: &str, dest: &Path) -> std::io::Result<()> {
        self.pending.push((url.to_string(), dest.to_path_buf()));
        self.save()
    }
    /// Runs every queued download in order (blocking). A finished one leaves the queue; a failed one stays for the next
    /// run. Returns each url with its result.
    pub fn run_pending(&mut self, session: &URLSession) -> Vec<(String, Result<(), URLError>)> {
        let mut results = Vec::new();
        let mut left = Vec::new();
        for (url, dest) in std::mem::take(&mut self.pending) {
            let r = download_to(session, &url, &dest);
            if r.is_err() {
                left.push((url.clone(), dest));
            }
            results.push((url, r));
        }
        self.pending = left;
        let _ = self.save();
        results
    }
}

fn download_to(session: &URLSession, url: &str, dest: &Path) -> Result<(), URLError> {
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| URLError::FileError(e.to_string()))?;
    let cancel = AtomicBool::new(false);
    let mut write_error = None;
    let resp = session.bytes(&URLRequest::new(url), &cancel, &mut |_| {}, &mut |c| match file.write_all(c) {
        Ok(()) => true,
        Err(e) => {
            write_error = Some(e.to_string());
            false
        }
    });
    drop(file);
    let fail = |e: URLError| {
        let _ = std::fs::remove_file(&part);
        Err(e)
    };
    if let Some(e) = write_error {
        return fail(URLError::FileError(e));
    }
    match resp {
        Ok(r) if (200..300).contains(&r.status_code) => std::fs::rename(&part, dest).map_err(|e| URLError::FileError(e.to_string())),
        Ok(r) => fail(URLError::BadServerResponse(format!("status {}", r.status_code))),
        Err(e) => fail(e),
    }
}
