//! `URLSessionConfiguration` (default, ephemeral, background), a configured session that wires the cookie jar, the
//! credential storage, the disk `URLCache` and authentication challenges into requests, `URLSessionTaskMetrics`, and
//! background `URLSession` sessions with download resume data (Foundation parity).
//!
//! `ConfiguredSession::data` is blocking (call it from a background thread). Per request it: applies the cache policy,
//! attaches cookies, sends, stores `Set-Cookie`, and on a 401 with `WWW-Authenticate: Basic` or `Digest` asks the
//! delegate (`urlSession(_:didReceive:completionHandler:)`) and retries once with the chosen credential. A Digest
//! challenge is parsed by `http_digest` (MD5, quality of protection `auth` or none) and answered with nonce count 1
//! and a fresh client nonce; an unparsable challenge, a declined one or a second 401 returns the 401 unchanged.
//! A background configuration builds a `BackgroundSession` (`URLSessionConfiguration::background_session`); building
//! one with the same identifier picks the unfinished downloads up again.
//!
//! `BackgroundConfiguration` mirrors `URLSessionConfiguration.background(withIdentifier:)`, `ResumeData` the
//! `NSURLSessionDownloadTaskResumeData` blob, and `BackgroundSession` a session whose download tasks are persisted
//! under a directory (the app support dir) so they can be restored by session identifier after the app quit. Work
//! goes through `URLSession::bytes`; a resumed task sends `Range` and `If-Range`, and a `200` instead of `206`
//! restarts from zero.

use crate::http_auth::{parse_www_authenticate, AuthChallenge, AuthDisposition, URLCredentialStorage, URLProtectionSpace};
use crate::http_cookies::HTTPCookieStorage;
use crate::http_digest::{authorization, parse_digest_challenge, DigestAnswer};
use crate::url::{HTTPURLResponse, URLError, URLRequest, URLSession};
use crate::url_cache::{CacheDecision, CachePolicy, URLCache};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `NSURLSessionDownloadTaskResumeData`: the key of the resume blob in an error's user info.
pub const URL_SESSION_DOWNLOAD_TASK_RESUME_DATA: &str = "NSURLSessionDownloadTaskResumeData";

const RESUME_MAGIC: &str = "bifrost-resume-v1";

/// `URLSessionConfiguration.background(withIdentifier:)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundConfiguration {
    pub identifier: String,
    /// `isDiscretionary`: the system may defer the transfer to a good moment.
    pub is_discretionary: bool,
    /// `sessionSendsLaunchEvents`: relaunch the app when transfers finish.
    pub sessions_send_launch_events: bool,
}

impl BackgroundConfiguration {
    /// `background(withIdentifier:)`.
    pub fn background(identifier: &str) -> Self {
        BackgroundConfiguration { identifier: identifier.to_string(), is_discretionary: false, sessions_send_launch_events: true }
    }
}

/// What a partial download needs to continue: the URL, the validator and the bytes already on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeData {
    pub url: String,
    pub etag: Option<String>,
    pub bytes_received: u64,
}

impl ResumeData {
    /// The opaque blob (`Data`) handed to the app.
    pub fn encode(&self) -> Vec<u8> {
        format!("{RESUME_MAGIC}\n{}\n{}\n{}\n", escape(&self.url), escape(self.etag.as_deref().unwrap_or("")), self.bytes_received).into_bytes()
    }
    /// `None` when the blob is not resume data from `encode`.
    pub fn decode(data: &[u8]) -> Option<ResumeData> {
        let text = std::str::from_utf8(data).ok()?;
        let mut lines = text.split('\n');
        if lines.next()? != RESUME_MAGIC {
            return None;
        }
        let url = unescape(lines.next()?);
        let etag = unescape(lines.next()?);
        let bytes_received = lines.next()?.parse().ok()?;
        if url.is_empty() {
            return None;
        }
        Some(ResumeData { url, etag: (!etag.is_empty()).then_some(etag), bytes_received })
    }
}

/// `URLSessionDownloadDelegate`.
pub trait DownloadDelegate {
    /// `urlSession(_:downloadTask:didWriteData:totalBytesWritten:totalBytesExpectedToWrite:)`.
    fn did_write_data(&mut self, task: u64, written: u64, total_written: u64, total_expected: Option<u64>);
    /// `urlSession(_:downloadTask:didFinishDownloadingTo:)`.
    fn did_finish_downloading_to(&mut self, task: u64, location: &Path);
    /// `urlSession(_:task:didCompleteWithError:)`; `Ok` carries the final response.
    fn did_complete(&mut self, task: u64, result: Result<HTTPURLResponse, URLError>);
}

/// A persisted download task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: u64,
    pub url: String,
    pub destination: PathBuf,
    pub etag: Option<String>,
    pub bytes_received: u64,
}

struct State {
    tasks: BTreeMap<u64, TaskRecord>,
    cancels: BTreeMap<u64, Arc<AtomicBool>>,
    next_id: u64,
}

/// A background session: tasks are saved to `<dir>/<identifier>.tasks` after every change.
#[derive(Clone)]
pub struct BackgroundSession {
    pub configuration: BackgroundConfiguration,
    session: URLSession,
    dir: PathBuf,
    state: Arc<Mutex<State>>,
}

impl BackgroundSession {
    /// Opens (or restores) the session named by `configuration.identifier` under `dir`.
    pub fn open(configuration: BackgroundConfiguration, session: URLSession, dir: &Path) -> BackgroundSession {
        let mut tasks = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(table_path(dir, &configuration.identifier)) {
            for rec in text.lines().filter_map(parse_record) {
                tasks.insert(rec.id, rec);
            }
        }
        let next_id = tasks.keys().next_back().map_or(1, |i| i + 1);
        BackgroundSession { configuration, session, dir: dir.to_path_buf(), state: Arc::new(Mutex::new(State { tasks, cancels: BTreeMap::new(), next_id })) }
    }

    /// `getTasksWithCompletionHandler`: tasks not yet finished, by id.
    pub fn pending_tasks(&self) -> Vec<TaskRecord> {
        self.state.lock().unwrap().tasks.values().cloned().collect()
    }

    /// `downloadTask(with:)`: registers the task; `run` performs it.
    pub fn download_task(&self, url: &str, destination: &Path) -> u64 {
        self.add(TaskRecord { id: 0, url: url.to_string(), destination: destination.to_path_buf(), etag: None, bytes_received: 0 })
    }

    /// `downloadTask(withResumeData:)`. `None` when the blob does not decode.
    pub fn download_task_with_resume_data(&self, data: &[u8], destination: &Path) -> Option<u64> {
        let r = ResumeData::decode(data)?;
        Some(self.add(TaskRecord { id: 0, url: r.url, destination: destination.to_path_buf(), etag: r.etag, bytes_received: r.bytes_received }))
    }

    /// `cancel(byProducingResumeData:)`: stops a running task and returns its resume blob when anything was
    /// received. The partial file and the task are kept, so the blob can also be resumed with `run`.
    pub fn cancel_by_producing_resume_data(&self, id: u64) -> Option<Vec<u8>> {
        let st = self.state.lock().unwrap();
        st.cancels.get(&id)?.store(true, Ordering::Relaxed);
        let rec = st.tasks.get(&id)?;
        let on_disk = std::fs::metadata(part_path(&rec.destination)).map_or(0, |m| m.len());
        (on_disk > 0).then(|| ResumeData { url: rec.url.clone(), etag: rec.etag.clone(), bytes_received: on_disk }.encode())
    }

    /// `cancel()`: stops the task, drops it and removes its partial file.
    pub fn cancel(&self, id: u64) {
        let mut st = self.state.lock().unwrap();
        if let Some(flag) = st.cancels.remove(&id) {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(rec) = st.tasks.remove(&id) {
            let _ = std::fs::remove_file(part_path(&rec.destination));
        }
        self.save(&st);
    }

    /// Runs task `id` to the end on the calling thread (call from a background queue).
    pub fn run(&self, id: u64, delegate: &mut dyn DownloadDelegate) {
        let (rec, flag) = {
            let mut st = self.state.lock().unwrap();
            let Some(rec) = st.tasks.get(&id).cloned() else { return };
            let flag = Arc::new(AtomicBool::new(false));
            st.cancels.insert(id, flag.clone());
            (rec, flag)
        };
        let result = self.transfer(&rec, &flag, delegate);
        let mut st = self.state.lock().unwrap();
        st.cancels.remove(&id);
        match &result {
            Err(URLError::Cancelled) => {}
            _ => {
                st.tasks.remove(&id);
                if !matches!(&result, Ok(r) if (200..300).contains(&r.status_code)) {
                    let _ = std::fs::remove_file(part_path(&rec.destination));
                }
            }
        }
        self.save(&st);
        drop(st);
        delegate.did_complete(id, result);
    }

    fn transfer(&self, rec: &TaskRecord, cancel: &AtomicBool, delegate: &mut dyn DownloadDelegate) -> Result<HTTPURLResponse, URLError> {
        let part = part_path(&rec.destination);
        let on_disk = std::fs::metadata(&part).map_or(0, |m| m.len());
        let start = if rec.bytes_received > 0 && on_disk >= rec.bytes_received { rec.bytes_received } else { 0 };
        let mut req = URLRequest::new(&rec.url);
        if start > 0 {
            req.set_header("Range", &format!("bytes={start}-"));
            if let Some(tag) = &rec.etag {
                req.set_header("If-Range", tag);
            }
        }
        struct Run<'a> {
            ok: bool,
            offset: u64,
            expected: Option<u64>,
            file: Option<std::fs::File>,
            err: Option<String>,
            delegate: &'a mut dyn DownloadDelegate,
        }
        let run = RefCell::new(Run { ok: false, offset: start, expected: None, file: None, err: None, delegate });
        let id = rec.id;
        let result = self.session.bytes(
            &req,
            cancel,
            &mut |r| {
                let mut s = run.borrow_mut();
                s.ok = (200..300).contains(&r.status_code);
                let len = r.value("Content-Length").and_then(|v| v.parse::<u64>().ok());
                if r.status_code == 206 {
                    s.expected = len.map(|l| l + start);
                } else {
                    // 200 ignores the range: restart from zero.
                    s.offset = 0;
                    s.expected = len;
                }
                if s.ok {
                    let etag = r.value("ETag").map(str::to_string).or_else(|| rec.etag.clone());
                    let mut st = self.state.lock().unwrap();
                    if let Some(t) = st.tasks.get_mut(&id) {
                        t.etag = etag;
                        t.bytes_received = s.offset;
                    }
                    self.save(&st);
                }
            },
            &mut |chunk| {
                let mut s = run.borrow_mut();
                if !s.ok {
                    return true;
                }
                if s.file.is_none() {
                    let mut o = OpenOptions::new();
                    o.create(true);
                    if s.offset == 0 {
                        o.write(true).truncate(true);
                    } else {
                        o.append(true);
                    }
                    match o.open(&part) {
                        Ok(f) => s.file = Some(f),
                        Err(e) => {
                            s.err = Some(e.to_string());
                            return false;
                        }
                    }
                }
                if let Err(e) = s.file.as_mut().unwrap().write_all(chunk) {
                    s.err = Some(e.to_string());
                    return false;
                }
                s.offset += chunk.len() as u64;
                let (total, expected) = (s.offset, s.expected);
                s.delegate.did_write_data(id, chunk.len() as u64, total, expected);
                let mut st = self.state.lock().unwrap();
                if let Some(t) = st.tasks.get_mut(&id) {
                    t.bytes_received = total;
                }
                true
            },
        );
        let mut s = run.into_inner();
        drop(s.file.take());
        if let Some(e) = s.err {
            return Err(URLError::FileError(e));
        }
        if matches!(result, Err(URLError::Cancelled)) {
            let st = self.state.lock().unwrap();
            self.save(&st);
        }
        let resp = result?;
        if s.ok {
            std::fs::rename(&part, &rec.destination).map_err(|e| URLError::FileError(e.to_string()))?;
            s.delegate.did_finish_downloading_to(id, &rec.destination);
        }
        Ok(resp)
    }

    fn add(&self, mut rec: TaskRecord) -> u64 {
        let mut st = self.state.lock().unwrap();
        rec.id = st.next_id;
        st.next_id += 1;
        let id = rec.id;
        st.tasks.insert(id, rec);
        self.save(&st);
        id
    }

    fn save(&self, st: &State) {
        let text: String = st.tasks.values().map(format_record).collect();
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(table_path(&self.dir, &self.configuration.identifier), text);
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

fn table_path(dir: &Path, identifier: &str) -> PathBuf {
    let safe: String = identifier.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    dir.join(format!("{safe}.tasks"))
}

fn format_record(r: &TaskRecord) -> String {
    format!("{}\t{}\t{}\t{}\t{}\n", r.id, escape(&r.url), escape(&r.destination.to_string_lossy()), escape(r.etag.as_deref().unwrap_or("")), r.bytes_received)
}

fn parse_record(line: &str) -> Option<TaskRecord> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() != 5 {
        return None;
    }
    let etag = unescape(f[3]);
    Some(TaskRecord { id: f[0].parse().ok()?, url: unescape(f[1]), destination: PathBuf::from(unescape(f[2])), etag: (!etag.is_empty()).then_some(etag), bytes_received: f[4].parse().ok()? })
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// The three kinds of configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationKind {
    Default,
    Ephemeral,
    Background(BackgroundConfiguration),
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
        URLSessionConfiguration { kind: ConfigurationKind::Background(BackgroundConfiguration::background(identifier)), ..Self::default_configuration(None) }
    }
    /// `identifier`: set for background configurations only.
    pub fn identifier(&self) -> Option<&str> {
        match &self.kind {
            ConfigurationKind::Background(c) => Some(&c.identifier),
            _ => None,
        }
    }
    /// The background configuration (`isDiscretionary`, `sessionSendsLaunchEvents`), for background kinds only.
    pub fn background_configuration(&self) -> Option<&BackgroundConfiguration> {
        match &self.kind {
            ConfigurationKind::Background(c) => Some(c),
            _ => None,
        }
    }
    /// Opens (or restores) the background session this configuration names, persisting its tasks under `dir`.
    /// `None` for default and ephemeral configurations.
    pub fn background_session(&self, session: URLSession, dir: &Path) -> Option<BackgroundSession> {
        self.background_configuration().map(|c| BackgroundSession::open(c.clone(), session, dir))
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

    /// The request to retry after a Basic or Digest challenge, or `None` when the delegate declines, the scheme is
    /// neither, or a Digest challenge cannot be answered (unparsable, another algorithm, no client nonce).
    fn answer_challenge(&self, req: &URLRequest, resp: &HTTPURLResponse) -> Option<URLRequest> {
        let header = resp.value("WWW-Authenticate")?;
        let challenge = parse_www_authenticate(header)?;
        if challenge.scheme != "basic" && challenge.scheme != "digest" {
            return None;
        }
        let digest = if challenge.scheme == "digest" { Some(parse_digest_challenge(header)?) } else { None };
        let space = URLProtectionSpace::for_challenge(&req.url, &challenge)?;
        let storage = self.configuration.url_credential_storage.as_ref()?;
        let ch = AuthChallenge::new(space.clone());
        let disposition = match &self.delegate {
            Some(d) => d.did_receive(&ch),
            None => ch.disposition(&storage.lock().unwrap()),
        };
        let AuthDisposition::UseCredential(cred) = disposition else { return None };
        let mut retry = req.clone();
        match digest {
            None => cred.apply_to(&mut retry),
            Some(d) => {
                // RFC 7616 section 3.4: the first answer to a nonce has nc=00000001; the single retry is its only use.
                let cnonce: String = crate::entropy::bytes(8).ok()?.iter().map(|b| format!("{b:02x}")).collect();
                let uri = crate::url::request_target(&crate::url::parse_url(&req.url).ok()?, &req.url, false);
                let answer = DigestAnswer { user: &cred.user, password: &cred.password, method: &req.method, uri: &uri, cnonce: &cnonce, nonce_count: 1 };
                retry.set_header("Authorization", &authorization(&d, &answer));
            }
        }
        if cred.persistence != crate::http_auth::CredentialPersistence::None {
            storage.lock().unwrap().set_credential(cred, space);
        }
        Some(retry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::deildu::http::{Stream, Transport};
    use std::io::{self, Read};

    /// Serves one canned reply per connection and logs the request text.
    struct Fake {
        replies: Mutex<Vec<Vec<u8>>>,
        log: Arc<Mutex<Vec<String>>>,
    }
    struct Conn {
        reply: io::Cursor<Vec<u8>>,
        req: Vec<u8>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl Read for Conn {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.reply.read(b)
        }
    }
    impl Write for Conn {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.req.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.req.ends_with(b"\r\n\r\n") {
                self.log.lock().unwrap().push(String::from_utf8_lossy(&self.req).to_ascii_lowercase());
                self.req.clear();
            }
            Ok(())
        }
    }
    impl Transport for Fake {
        fn connect(&self, _: &str, _: u16, _: bool) -> io::Result<Box<dyn Stream>> {
            let reply = self.replies.lock().unwrap().remove(0);
            Ok(Box::new(Conn { reply: io::Cursor::new(reply), req: Vec::new(), log: self.log.clone() }))
        }
    }

    fn session(replies: Vec<Vec<u8>>) -> (URLSession, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (URLSession::with_transport(Arc::new(Fake { replies: Mutex::new(replies), log: log.clone() }), None), log)
    }

    fn reply(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut v = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        v.extend_from_slice(body);
        v
    }

    #[derive(Default)]
    struct Rec {
        writes: Vec<(u64, u64, Option<u64>)>,
        finished: Option<PathBuf>,
        completed: Option<Result<u16, URLError>>,
    }
    impl DownloadDelegate for Rec {
        fn did_write_data(&mut self, _: u64, w: u64, t: u64, e: Option<u64>) {
            self.writes.push((w, t, e));
        }
        fn did_finish_downloading_to(&mut self, _: u64, p: &Path) {
            self.finished = Some(p.to_path_buf());
        }
        fn did_complete(&mut self, _: u64, r: Result<HTTPURLResponse, URLError>) {
            self.completed = Some(r.map(|x| x.status_code));
        }
    }

    #[test]
    fn configuration_round_trips_identifier_and_flags() {
        let mut c = BackgroundConfiguration::background("com.example.dl");
        c.is_discretionary = true;
        assert_eq!(c.identifier, "com.example.dl");
        assert!(c.is_discretionary && c.sessions_send_launch_events);
    }

    #[test]
    fn resume_data_encodes_and_decodes() {
        let r = ResumeData { url: "http://h/a b\nc".into(), etag: Some("\"v1\"".into()), bytes_received: 42 };
        assert_eq!(ResumeData::decode(&r.encode()), Some(r));
        let none = ResumeData { url: "http://h/x".into(), etag: None, bytes_received: 0 };
        assert_eq!(ResumeData::decode(&none.encode()), Some(none));
        assert_eq!(ResumeData::decode(b"junk"), None);
    }

    #[test]
    fn fresh_download_reports_progress_and_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let (s, log) = session(vec![reply("200 OK", "ETag: \"v1\"\r\n", b"hello")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let dest = dir.path().join("out.bin");
        let id = bg.download_task("http://h/f", &dest);
        let mut d = Rec::default();
        bg.run(id, &mut d);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(d.writes.last(), Some(&(5, 5, Some(5))));
        assert_eq!(d.finished, Some(dest));
        assert_eq!(d.completed, Some(Ok(200)));
        assert!(!log.lock().unwrap()[0].contains("range:"));
        assert!(bg.pending_tasks().is_empty());
    }

    #[test]
    fn resumed_task_sends_range_and_if_range_and_appends_on_206() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), b"hel").unwrap();
        let (s, log) = session(vec![reply("206 Partial Content", "", b"lo")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let blob = ResumeData { url: "http://h/f".into(), etag: Some("\"v1\"".into()), bytes_received: 3 }.encode();
        let id = bg.download_task_with_resume_data(&blob, &dest).unwrap();
        let mut d = Rec::default();
        bg.run(id, &mut d);
        let req = log.lock().unwrap()[0].clone();
        assert!(req.contains("range: bytes=3-") && req.contains("if-range: \"v1\""));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(d.writes.last(), Some(&(2, 5, Some(5))));
    }

    #[test]
    fn status_200_instead_of_206_restarts_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), b"old").unwrap();
        let (s, _) = session(vec![reply("200 OK", "", b"fresh")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let blob = ResumeData { url: "http://h/f".into(), etag: None, bytes_received: 3 }.encode();
        let id = bg.download_task_with_resume_data(&blob, &dest).unwrap();
        bg.run(id, &mut Rec::default());
        assert_eq!(std::fs::read(&dest).unwrap(), b"fresh");
    }

    #[test]
    fn pending_tasks_restore_by_session_identifier() {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) = session(vec![]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("one"), s.clone(), dir.path());
        let id = bg.download_task("http://h/f", &dir.path().join("o"));
        let again = BackgroundSession::open(BackgroundConfiguration::background("one"), s.clone(), dir.path());
        assert_eq!(again.pending_tasks().len(), 1);
        assert_eq!(again.pending_tasks()[0].id, id);
        let other = BackgroundSession::open(BackgroundConfiguration::background("two"), s, dir.path());
        assert!(other.pending_tasks().is_empty());
        assert!(again.download_task("http://h/g", &dir.path().join("p")) > id);
    }

    #[test]
    fn cancel_by_producing_resume_data_needs_received_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) = session(vec![]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let dest = dir.path().join("o");
        let id = bg.download_task("http://h/f", &dest);
        bg.state.lock().unwrap().cancels.insert(id, Arc::new(AtomicBool::new(false)));
        assert_eq!(bg.cancel_by_producing_resume_data(id), None);
        std::fs::write(part_path(&dest), b"abcd").unwrap();
        let r = ResumeData::decode(&bg.cancel_by_producing_resume_data(id).unwrap()).unwrap();
        assert_eq!((r.url.as_str(), r.bytes_received), ("http://h/f", 4));
    }

    /// Answers every challenge with one disposition and counts the questions.
    struct Answer {
        disposition: AuthDisposition,
        asked: Mutex<u32>,
    }
    impl URLSessionTaskDelegate for Answer {
        fn did_receive(&self, _: &AuthChallenge) -> AuthDisposition {
            *self.asked.lock().unwrap() += 1;
            self.disposition.clone()
        }
    }

    fn challenged(delegate: Option<Arc<dyn URLSessionTaskDelegate>>, storage: Arc<Mutex<URLCredentialStorage>>, replies: Vec<Vec<u8>>) -> (ConfiguredSession, Arc<Mutex<Vec<String>>>) {
        let (s, log) = session(replies);
        let mut c = URLSessionConfiguration::default_configuration(None);
        c.url_credential_storage = Some(storage);
        (ConfiguredSession::new(c, s, delegate), log)
    }

    /// A 401 that asks for `scheme` with realm "r", then a 200 with body "ok".
    fn challenge_then_ok(scheme: &str) -> Vec<Vec<u8>> {
        vec![reply("401 Unauthorized", &format!("WWW-Authenticate: {scheme} realm=\"r\"\r\n"), b""), reply("200 OK", "", b"ok")]
    }

    fn space() -> URLProtectionSpace {
        URLProtectionSpace { host: "h".into(), port: 80, realm: Some("r".into()) }
    }

    #[test]
    fn a_basic_challenge_is_answered_from_the_stored_credential() {
        use crate::http_auth::{CredentialPersistence, URLCredential};
        let storage = Arc::new(Mutex::new(URLCredentialStorage::new()));
        storage.lock().unwrap().set_credential(URLCredential::new("ola", "pw", CredentialPersistence::ForSession), space());
        let (cs, log) = challenged(None, storage, challenge_then_ok("Basic"));
        let (resp, body) = cs.data(&URLRequest::new("http://h/f")).unwrap();
        assert_eq!((resp.status_code, body.as_slice()), (200, &b"ok"[..]));
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2, "one retry");
        // the fake transport logs the request lower-cased, so the base64 of "ola:pw" is compared in that form
        assert!(log[1].contains("authorization: basic b2xhonb3"), "{}", log[1]);
    }

    #[test]
    fn a_delegate_credential_is_sent_and_kept_when_it_persists() {
        use crate::http_auth::{CredentialPersistence, URLCredential};
        let storage = Arc::new(Mutex::new(URLCredentialStorage::new()));
        let answer = Arc::new(Answer { disposition: AuthDisposition::UseCredential(URLCredential::new("ana", "secret", CredentialPersistence::Permanent)), asked: Mutex::new(0) });
        let (cs, log) = challenged(Some(answer.clone()), storage.clone(), challenge_then_ok("Basic"));
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 200);
        assert_eq!(*answer.asked.lock().unwrap(), 1);
        assert!(log.lock().unwrap()[1].contains("authorization: basic "));
        assert_eq!(storage.lock().unwrap().credential(&space()).map(|c| c.user.clone()), Some("ana".to_string()));
    }

    #[test]
    fn a_credential_that_is_not_kept_is_sent_but_not_stored() {
        use crate::http_auth::{CredentialPersistence, URLCredential};
        let storage = Arc::new(Mutex::new(URLCredentialStorage::new()));
        let answer = Arc::new(Answer { disposition: AuthDisposition::UseCredential(URLCredential::new("ana", "secret", CredentialPersistence::None)), asked: Mutex::new(0) });
        let (cs, log) = challenged(Some(answer), storage.clone(), challenge_then_ok("Basic"));
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 200);
        assert!(log.lock().unwrap()[1].contains("authorization: basic "));
        assert!(storage.lock().unwrap().is_empty());
    }

    #[test]
    fn a_declined_challenge_leaves_the_401_to_the_caller() {
        let answer = Arc::new(Answer { disposition: AuthDisposition::PerformDefaultHandling, asked: Mutex::new(0) });
        let replies = vec![reply("401 Unauthorized", "WWW-Authenticate: Basic realm=\"r\"\r\n", b"")];
        let (cs, log) = challenged(Some(answer), Arc::new(Mutex::new(URLCredentialStorage::new())), replies);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 401);
        assert_eq!(log.lock().unwrap().len(), 1, "no retry");
    }

    fn digest_challenge_reply(extra: &str) -> Vec<u8> {
        reply("401 Unauthorized", &format!("WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\"{extra}\r\n"), b"")
    }

    fn digest_session(replies: Vec<Vec<u8>>, with_credential: bool) -> (ConfiguredSession, Arc<Mutex<Vec<String>>>) {
        use crate::http_auth::{CredentialPersistence, URLCredential};
        let storage = Arc::new(Mutex::new(URLCredentialStorage::new()));
        if with_credential {
            storage.lock().unwrap().set_credential(URLCredential::new("ola", "pw", CredentialPersistence::ForSession), space());
        }
        challenged(None, storage, replies)
    }

    /// The value of `key=` in a lower-cased Authorization line, quotes removed.
    fn field(request: &str, key: &str) -> String {
        let line = request.lines().find(|l| l.starts_with("authorization:")).expect("authorization header");
        let at = line.find(&format!(" {key}=")).unwrap_or_else(|| panic!("{key} in {line}")) + key.len() + 2;
        line[at..].split(',').next().unwrap().trim().trim_matches('"').to_string()
    }

    #[test]
    fn a_digest_challenge_is_answered_with_the_matching_response_digest() {
        use crate::http_digest::{response_digest, DigestChallenge};
        let (cs, log) = digest_session(vec![digest_challenge_reply(", qop=\"auth\""), reply("200 OK", "", b"ok")], true);
        let (resp, body) = cs.data(&URLRequest::new("http://h/f?x=1")).unwrap();
        assert_eq!((resp.status_code, body.as_slice()), (200, &b"ok"[..]));
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert!(!log[0].contains("authorization:"));
        let challenge = DigestChallenge { realm: "r".into(), nonce: "n1".into(), opaque: None, qop_auth: true };
        let cnonce = field(&log[1], "cnonce");
        assert_eq!(cnonce.len(), 16, "a fresh client nonce");
        assert_eq!(field(&log[1], "nc"), "00000001");
        assert_eq!(field(&log[1], "uri"), "/f?x=1");
        let answer = crate::http_digest::DigestAnswer { user: "ola", password: "pw", method: "GET", uri: "/f?x=1", cnonce: &cnonce, nonce_count: 1 };
        assert_eq!(field(&log[1], "response"), response_digest(&challenge, &answer));
    }

    #[test]
    fn a_digest_challenge_without_qop_has_no_nonce_count() {
        let (cs, log) = digest_session(vec![digest_challenge_reply(""), reply("200 OK", "", b"ok")], true);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 200);
        let log = log.lock().unwrap();
        assert!(!log[1].contains("nc="), "{}", log[1]);
        assert!(log[1].contains("authorization: digest username=\"ola\""), "{}", log[1]);
    }

    #[test]
    fn a_digest_challenge_without_a_credential_returns_the_401_after_one_request() {
        let (cs, log) = digest_session(vec![digest_challenge_reply(", qop=\"auth\"")], false);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 401);
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_second_digest_401_stops_after_two_requests() {
        let (cs, log) = digest_session(vec![digest_challenge_reply(", qop=\"auth\""), digest_challenge_reply(", qop=\"auth\", stale=true")], true);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 401);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn an_unparsable_or_unsupported_digest_challenge_is_not_retried() {
        for header in ["Digest realm=\"r\"", "Digest realm=\"r\", nonce=\"n\", algorithm=SHA-256"] {
            let replies = vec![reply("401 Unauthorized", &format!("WWW-Authenticate: {header}\r\n"), b"")];
            let (cs, log) = digest_session(replies, true);
            assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 401);
            assert_eq!(log.lock().unwrap().len(), 1, "{header}");
        }
    }

    #[test]
    fn a_digest_credential_from_the_delegate_is_kept_when_it_persists() {
        use crate::http_auth::{CredentialPersistence, URLCredential};
        let storage = Arc::new(Mutex::new(URLCredentialStorage::new()));
        let answer = Arc::new(Answer { disposition: AuthDisposition::UseCredential(URLCredential::new("ana", "secret", CredentialPersistence::Permanent)), asked: Mutex::new(0) });
        let (cs, _) = challenged(Some(answer.clone()), storage.clone(), vec![digest_challenge_reply(""), reply("200 OK", "", b"ok")]);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 200);
        assert_eq!(*answer.asked.lock().unwrap(), 1);
        assert_eq!(storage.lock().unwrap().credential(&space()).map(|c| c.user.clone()), Some("ana".to_string()));
    }

    #[test]
    fn an_unknown_scheme_is_not_answered() {
        let (cs, log) = digest_session(vec![reply("401 Unauthorized", "WWW-Authenticate: Bearer realm=\"r\"\r\n", b"")], true);
        assert_eq!(cs.data(&URLRequest::new("http://h/f")).unwrap().0.status_code, 401);
        assert_eq!(log.lock().unwrap().len(), 1);
    }
}
