//! `URLSession` parity: `URLSession::shared()`, `URLRequest`, `data_task`, `download_task`, `bytes` (streaming) and
//! a memory `URLCache` that honours `Cache-Control: max-age|no-store|no-cache` and `ETag`.
//!
//! Transport is the shell's rustls (graviola) stack, `springboard::deildu::http::Tls`. Completions run on the main
//! queue (`dispatch::DispatchQueue::main`); the work runs on a `UserInitiated` global queue. Plain `http://` works
//! too (lab servers, tests); redirects (301/302/303/307/308) are followed up to 5 hops.

use crate::dispatch::{DispatchQueue, Qos};
use springboard::deildu::http::{Stream, Tls, Transport};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_HEADER_LEN: usize = 64 << 10;
const MAX_REDIRECTS: usize = 5;

/// `URLError` (the cases that matter here).
#[derive(Clone, Debug, PartialEq)]
pub enum URLError {
    BadURL(String),
    CannotConnect(String),
    TimedOut,
    Cancelled,
    BadServerResponse(String),
    TooManyRedirects,
    FileError(String),
}

impl std::fmt::Display for URLError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            URLError::BadURL(s) => write!(f, "Bad URL: {s}"),
            URLError::CannotConnect(s) => write!(f, "Cannot connect: {s}"),
            URLError::TimedOut => write!(f, "The request timed out"),
            URLError::Cancelled => write!(f, "The request was cancelled"),
            URLError::BadServerResponse(s) => write!(f, "Bad server response: {s}"),
            URLError::TooManyRedirects => write!(f, "Too many redirects"),
            URLError::FileError(s) => write!(f, "File error: {s}"),
        }
    }
}

/// `URLRequest`.
#[derive(Clone, Debug)]
pub struct URLRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl URLRequest {
    pub fn new(url: &str) -> Self {
        URLRequest { url: url.to_string(), method: "GET".into(), headers: Vec::new(), body: None, timeout: Duration::from_secs(60) }
    }
    pub fn post(url: &str, body: Vec<u8>) -> Self {
        URLRequest { method: "POST".into(), body: Some(body), ..Self::new(url) }
    }
    /// `setValue(_:forHTTPHeaderField:)`: replaces a header of the same name.
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
    }
}

/// `HTTPURLResponse`.
#[derive(Clone, Debug, PartialEq)]
pub struct HTTPURLResponse {
    pub url: String,
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
}

impl HTTPURLResponse {
    /// `value(forHTTPHeaderField:)`, case-insensitive.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

type Reply = Result<(HTTPURLResponse, Vec<u8>), URLError>;

struct CacheEntry {
    response: HTTPURLResponse,
    body: Vec<u8>,
    stored: Instant,
    max_age: Duration,
    etag: Option<String>,
}

fn max_age_of(r: &HTTPURLResponse) -> Duration {
    let cc = r.value("Cache-Control").unwrap_or("").to_ascii_lowercase();
    if cc.contains("no-cache") {
        return Duration::ZERO;
    }
    cc.split(',').find_map(|d| d.trim().strip_prefix("max-age=").and_then(|v| v.trim().parse::<u64>().ok())).map(Duration::from_secs).unwrap_or(Duration::ZERO)
}

/// `URLCache` (memory only): GET 200 responses keyed by URL, bounded by `capacity` bytes (oldest dropped first).
pub struct URLCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    capacity: usize,
}

impl URLCache {
    pub fn new(capacity: usize) -> Self {
        URLCache { entries: Mutex::new(HashMap::new()), capacity }
    }
    pub fn remove_all(&self) {
        self.entries.lock().unwrap().clear();
    }
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn store(&self, resp: &HTTPURLResponse, body: &[u8]) {
        let cc = resp.value("Cache-Control").unwrap_or("").to_ascii_lowercase();
        if cc.contains("no-store") || body.len() > self.capacity {
            return;
        }
        let max_age = max_age_of(resp);
        let etag = resp.value("ETag").map(str::to_string);
        if max_age.is_zero() && etag.is_none() {
            return;
        }
        let mut map = self.entries.lock().unwrap();
        map.insert(resp.url.clone(), CacheEntry { response: resp.clone(), body: body.to_vec(), stored: Instant::now(), max_age, etag });
        let mut total: usize = map.values().map(|e| e.body.len()).sum();
        while total > self.capacity {
            let Some(oldest) = map.iter().min_by_key(|(_, e)| e.stored).map(|(k, _)| k.clone()) else { break };
            if let Some(e) = map.remove(&oldest) {
                total -= e.body.len();
            }
        }
    }
}

/// `URLSession`. Cheap to clone.
#[derive(Clone)]
pub struct URLSession {
    transport: Arc<dyn Transport>,
    cache: Option<Arc<URLCache>>,
}

/// `NSURLSessionTransferSizeUnknown`.
const TRANSFER_SIZE_UNKNOWN: i64 = -1;

/// Handle for cancelling a task (`URLSessionTask.cancel()`) and reading its upload counts.
#[derive(Clone)]
pub struct URLSessionTask {
    cancel: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
    expected: Arc<AtomicI64>,
}

impl Default for URLSessionTask {
    fn default() -> Self {
        URLSessionTask { cancel: Arc::default(), sent: Arc::default(), expected: Arc::new(AtomicI64::new(TRANSFER_SIZE_UNKNOWN)) }
    }
}

impl URLSessionTask {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// `countOfBytesSent`: request body bytes written to the connection so far (upload tasks; 0 otherwise).
    pub fn count_of_bytes_sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
    /// `countOfBytesExpectedToSend`: the body length, or -1 (`NSURLSessionTransferSizeUnknown`) when not known.
    pub fn count_of_bytes_expected_to_send(&self) -> i64 {
        self.expected.load(Ordering::Relaxed)
    }
}

/// A request body: bytes already in memory, or a file that is streamed from disk (`uploadTask(with:fromFile:)`).
enum Payload {
    Bytes(Vec<u8>),
    File { path: std::path::PathBuf, len: u64 },
}

/// Body chunk size: the unit of `didSendBodyData` progress and of cancel checks.
const UPLOAD_CHUNK: usize = 64 << 10;

impl Payload {
    /// Stats the file once, so a missing or unreadable file fails before any request is made.
    fn open(path: &std::path::Path) -> Result<Payload, URLError> {
        let len = std::fs::metadata(path).map_err(|e| URLError::FileError(e.to_string()))?.len();
        Ok(Payload::File { path: path.to_path_buf(), len })
    }
    fn len(&self) -> u64 {
        match self {
            Payload::Bytes(b) => b.len() as u64,
            Payload::File { len, .. } => *len,
        }
    }
    /// Writes the body in chunks. `check` runs before each chunk (cancel, deadline); `on_sent` gets each chunk's size.
    fn write_to<W: Write + ?Sized>(&self, conn: &mut W, check: &dyn Fn() -> Result<(), URLError>, on_sent: &mut dyn FnMut(u64)) -> Result<(), URLError> {
        let net = |e: std::io::Error| URLError::CannotConnect(e.to_string());
        match self {
            Payload::Bytes(b) => {
                for chunk in b.chunks(UPLOAD_CHUNK) {
                    check()?;
                    conn.write_all(chunk).map_err(net)?;
                    on_sent(chunk.len() as u64);
                }
            }
            Payload::File { path, len } => {
                let mut file = std::fs::File::open(path).map_err(|e| URLError::FileError(e.to_string()))?;
                let mut buf = vec![0u8; UPLOAD_CHUNK];
                let mut left = *len;
                while left > 0 {
                    check()?;
                    let want = buf.len().min(left as usize);
                    let n = file.read(&mut buf[..want]).map_err(|e| URLError::FileError(e.to_string()))?;
                    if n == 0 {
                        return Err(URLError::FileError("the file shrank during the upload".into()));
                    }
                    conn.write_all(&buf[..n]).map_err(net)?;
                    left -= n as u64;
                    on_sent(n as u64);
                }
            }
        }
        Ok(())
    }
}

/// The body a request carries: `httpBody` for data, download and streaming requests.
fn payload_of(req: &URLRequest) -> Option<Payload> {
    req.body.clone().map(Payload::Bytes)
}

impl URLSession {
    /// `URLSession.shared`: the real TLS transport and a 16 MiB memory cache.
    pub fn shared() -> URLSession {
        static SHARED: OnceLock<URLSession> = OnceLock::new();
        SHARED.get_or_init(|| URLSession { transport: Arc::new(Tls::new()), cache: Some(Arc::new(URLCache::new(16 << 20))) }).clone()
    }
    /// A session over any transport (tests), optionally with a cache.
    pub fn with_transport(transport: Arc<dyn Transport>, cache: Option<Arc<URLCache>>) -> URLSession {
        URLSession { transport, cache }
    }
    pub fn url_cache(&self) -> Option<&Arc<URLCache>> {
        self.cache.as_ref()
    }

    /// `dataTask(with:completionHandler:)`: `completion` runs on the main queue.
    pub fn data_task<F: FnOnce(Reply) + Send + 'static>(&self, req: URLRequest, completion: F) -> URLSessionTask {
        let task = URLSessionTask::default();
        let (this, flag) = (self.clone(), task.cancel.clone());
        DispatchQueue::global(Qos::UserInitiated).async_(move || {
            let r = this.data_blocking(req, &flag);
            DispatchQueue::main().async_(move || completion(r));
        });
        task
    }

    /// `downloadTask(with:)` into `dest` (written to `dest.part`, renamed on success). `progress(received, expected)`
    /// and `completion` run on the main queue; a non-2xx status completes with the response and writes nothing.
    pub fn download_task<P, F>(&self, req: URLRequest, dest: std::path::PathBuf, progress: P, completion: F) -> URLSessionTask
    where
        P: Fn(u64, Option<u64>) + Send + Sync + 'static,
        F: FnOnce(Result<HTTPURLResponse, URLError>) + Send + 'static,
    {
        let task = URLSessionTask::default();
        let (this, flag) = (self.clone(), task.cancel.clone());
        let progress = Arc::new(progress);
        DispatchQueue::global(Qos::UserInitiated).async_(move || {
            let r = this.download_blocking(req, &dest, &flag, progress);
            DispatchQueue::main().async_(move || completion(r));
        });
        task
    }

    /// `uploadTask(with:from:completionHandler:)`: sends `body` as the request body (`Content-Length` is set from it).
    /// `progress(bytes_in_chunk, total_bytes_sent, total_bytes_expected)` is the `didSendBodyData` call and runs on the
    /// main queue; `completion` runs on the main queue too. The request's own `body` is replaced by `body`.
    pub fn upload_task<P, F>(&self, req: URLRequest, body: Vec<u8>, progress: P, completion: F) -> URLSessionTask
    where
        P: Fn(u64, u64, u64) + Send + Sync + 'static,
        F: FnOnce(Reply) + Send + 'static,
    {
        self.spawn_upload(req, progress, completion, move || Ok(Payload::Bytes(body)))
    }

    /// `uploadTask(with:fromFile:completionHandler:)`: streams `file` as the request body in 64 KiB chunks. A file that
    /// cannot be stat'ed completes with `FileError` and no request is made. Progress as for `upload_task`.
    pub fn upload_task_from_file<P, F>(&self, req: URLRequest, file: std::path::PathBuf, progress: P, completion: F) -> URLSessionTask
    where
        P: Fn(u64, u64, u64) + Send + Sync + 'static,
        F: FnOnce(Reply) + Send + 'static,
    {
        self.spawn_upload(req, progress, completion, move || Payload::open(&file))
    }

    fn spawn_upload<P, F, S>(&self, req: URLRequest, progress: P, completion: F, source: S) -> URLSessionTask
    where
        P: Fn(u64, u64, u64) + Send + Sync + 'static,
        F: FnOnce(Reply) + Send + 'static,
        S: FnOnce() -> Result<Payload, URLError> + Send + 'static,
    {
        let task = URLSessionTask::default();
        let (this, handle) = (self.clone(), task.clone());
        let progress = Arc::new(progress);
        DispatchQueue::global(Qos::UserInitiated).async_(move || {
            let r = this.upload_with(&req, source, &handle, &mut |chunk, sent, total| {
                let p = progress.clone();
                DispatchQueue::main().async_(move || p(chunk, sent, total));
            });
            DispatchQueue::main().async_(move || completion(r));
        });
        task
    }

    /// `URLSession.bytes(for:)`: blocking, call from a background thread. `on_response` runs once with the final head;
    /// `on_chunk` gets decoded body bytes (any status) and returns false to stop (result `Cancelled`).
    pub fn bytes(
        &self,
        req: &URLRequest,
        cancel: &AtomicBool,
        on_response: &mut dyn FnMut(&HTTPURLResponse),
        on_chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<HTTPURLResponse, URLError> {
        self.fetch(req, payload_of(req).as_ref(), cancel, &mut |_| {}, on_response, on_chunk)
    }

    /// The blocking upload behind `upload_task*`: `source` yields the body (opening the file, for uploads from disk),
    /// `task` carries cancel and the counts, and `progress` gets every chunk (`didSendBodyData`).
    fn upload_with<S>(&self, req: &URLRequest, source: S, task: &URLSessionTask, progress: &mut dyn FnMut(u64, u64, u64)) -> Reply
    where
        S: FnOnce() -> Result<Payload, URLError>,
    {
        if task.is_cancelled() {
            return Err(URLError::Cancelled);
        }
        let payload = source()?;
        let expected = payload.len();
        task.expected.store(expected as i64, Ordering::Relaxed);
        let sent = task.sent.clone();
        let mut on_sent = |n: u64| {
            let total = sent.fetch_add(n, Ordering::Relaxed) + n;
            progress(n, total, expected);
        };
        let mut body = Vec::new();
        let resp = self.fetch(req, Some(&payload), &task.cancel, &mut on_sent, &mut |_| {}, &mut |c| {
            body.extend_from_slice(c);
            true
        })?;
        Ok((resp, body))
    }

    fn data_blocking(&self, mut req: URLRequest, cancel: &AtomicBool) -> Reply {
        let cache = self.cache.as_ref().filter(|_| req.method == "GET");
        if let Some(cache) = cache {
            if let Some(e) = cache.entries.lock().unwrap().get(&req.url) {
                if e.stored.elapsed() < e.max_age {
                    return Ok((e.response.clone(), e.body.clone()));
                }
                if let Some(t) = &e.etag {
                    req.set_header("If-None-Match", t);
                }
            }
        }
        let mut body = Vec::new();
        let payload = payload_of(&req);
        let resp = self.fetch(&req, payload.as_ref(), cancel, &mut |_| {}, &mut |_| {}, &mut |c| {
            body.extend_from_slice(c);
            true
        })?;
        if let Some(cache) = cache {
            if resp.status_code == 304 {
                if let Some(e) = cache.entries.lock().unwrap().get_mut(&req.url) {
                    e.stored = Instant::now();
                    let fresh = max_age_of(&resp);
                    if !fresh.is_zero() {
                        e.max_age = fresh;
                    }
                    return Ok((e.response.clone(), e.body.clone()));
                }
            } else if resp.status_code == 200 {
                cache.store(&resp, &body);
            }
        }
        Ok((resp, body))
    }

    fn download_blocking(&self, req: URLRequest, dest: &std::path::Path, cancel: &AtomicBool, progress: Arc<dyn Fn(u64, Option<u64>) + Send + Sync>) -> Result<HTTPURLResponse, URLError> {
        let part = dest.with_extension("part");
        let mut file = std::fs::File::create(&part).map_err(|e| URLError::FileError(e.to_string()))?;
        let mut received = 0u64;
        let expected: std::cell::Cell<Option<u64>> = std::cell::Cell::new(None);
        let mut werr = None;
        let ok = std::cell::Cell::new(false);
        let payload = payload_of(&req);
        let result = self.fetch(
            &req,
            payload.as_ref(),
            cancel,
            &mut |_| {},
            &mut |r| {
                ok.set((200..300).contains(&r.status_code));
                expected.set(r.value("Content-Length").and_then(|v| v.parse().ok()));
            },
            &mut |c| {
                if !ok.get() {
                    return true;
                }
                if let Err(e) = file.write_all(c) {
                    werr = Some(e.to_string());
                    return false;
                }
                received += c.len() as u64;
                let (p, r, x) = (progress.clone(), received, expected.get());
                DispatchQueue::main().async_(move || p(r, x));
                true
            },
        );
        drop(file);
        if let Some(e) = werr {
            let _ = std::fs::remove_file(&part);
            return Err(URLError::FileError(e));
        }
        match result {
            Ok(resp) if ok.get() => std::fs::rename(&part, dest).map(|_| resp).map_err(|e| URLError::FileError(e.to_string())),
            other => {
                let _ = std::fs::remove_file(&part);
                other
            }
        }
    }

    /// One request with redirects. Redirect bodies are skipped; `on_sent` sees each body chunk written, and
    /// `on_response`/`on_chunk` see the final reply.
    fn fetch(
        &self,
        req: &URLRequest,
        payload: Option<&Payload>,
        cancel: &AtomicBool,
        on_sent: &mut dyn FnMut(u64),
        on_response: &mut dyn FnMut(&HTTPURLResponse),
        on_chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<HTTPURLResponse, URLError> {
        let deadline = Instant::now() + req.timeout;
        let (mut url, mut method, mut payload) = (req.url.clone(), req.method.clone(), payload);
        for _ in 0..=MAX_REDIRECTS {
            let (resp, redirect) = self.once(&url, &method, &req.headers, payload, cancel, deadline, on_sent, on_response, on_chunk)?;
            match redirect {
                Some(loc) => {
                    url = resolve(&url, &loc)?;
                    if resp.status_code == 303 || (resp.status_code != 307 && resp.status_code != 308 && method == "POST") {
                        method = "GET".into();
                        payload = None;
                    }
                }
                None => return Ok(resp),
            }
        }
        Err(URLError::TooManyRedirects)
    }

    #[allow(clippy::too_many_arguments)]
    fn once(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
        payload: Option<&Payload>,
        cancel: &AtomicBool,
        deadline: Instant,
        on_sent: &mut dyn FnMut(u64),
        on_response: &mut dyn FnMut(&HTTPURLResponse),
        on_chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(HTTPURLResponse, Option<String>), URLError> {
        let p = parse_url(url)?;
        let mut conn = self.transport.connect(&p.host, p.port, p.tls).map_err(|e| URLError::CannotConnect(format!("{}: {e}", p.host)))?;
        let mut head = format!("{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: BifrostApp/1\r\nAccept: */*\r\nConnection: close\r\n", p.target, p.host_header);
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(b) = payload {
            head.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        head.push_str("\r\n");
        let check = || -> Result<(), URLError> {
            if cancel.load(Ordering::Relaxed) {
                Err(URLError::Cancelled)
            } else if Instant::now() > deadline {
                Err(URLError::TimedOut)
            } else {
                Ok(())
            }
        };
        conn.write_all(head.as_bytes()).map_err(|e| URLError::CannotConnect(e.to_string()))?;
        if let Some(b) = payload {
            b.write_to(&mut *conn, &check, on_sent)?;
        }
        conn.flush().map_err(|e| URLError::CannotConnect(e.to_string()))?;
        let read = |conn: &mut Box<dyn Stream>, buf: &mut [u8]| -> Result<usize, URLError> {
            conn.read(buf).map_err(|e| match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => URLError::TimedOut,
                _ => URLError::CannotConnect(e.to_string()),
            })
        };
        let mut buf = [0u8; 8192];
        let mut acc: Vec<u8> = Vec::new();
        let split = loop {
            check()?;
            let n = read(&mut conn, &mut buf)?;
            if n == 0 {
                return Err(URLError::BadServerResponse("connection closed before a response".into()));
            }
            acc.extend_from_slice(&buf[..n]);
            if let Some(i) = acc.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            if acc.len() > MAX_HEADER_LEN {
                return Err(URLError::BadServerResponse("headers too large".into()));
            }
        };
        let text = String::from_utf8_lossy(&acc[..split]).into_owned();
        let mut lines = text.lines();
        let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).ok_or_else(|| URLError::BadServerResponse("bad status line".into()))?;
        let hdrs: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        let resp = HTTPURLResponse { url: url.to_string(), status_code: status, headers: hdrs };
        let redirect = if matches!(status, 301 | 302 | 303 | 307 | 308) { resp.value("Location").map(str::to_string) } else { None };
        let chunked = resp.value("Transfer-Encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
        let length: Option<usize> = resp.value("Content-Length").and_then(|v| v.parse().ok());
        let no_body = method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status);
        if redirect.is_some() {
            return Ok((resp, redirect));
        }
        on_response(&resp);
        let mut dechunk = ChunkDecoder::default();
        let mut rest = acc[split + 4..].to_vec();
        let mut got = 0usize;
        loop {
            let mut data = if chunked { dechunk.feed(&rest) } else { std::mem::take(&mut rest) };
            rest.clear();
            if let (Some(l), false) = (length, chunked) {
                data.truncate(l.saturating_sub(got));
            }
            got += data.len();
            if !no_body && !data.is_empty() && !on_chunk(&data) {
                return Err(URLError::Cancelled);
            }
            if no_body || (chunked && dechunk.finished) || (!chunked && length.is_some_and(|l| got >= l)) {
                break;
            }
            check()?;
            let n = read(&mut conn, &mut buf)?;
            if n == 0 {
                break;
            }
            rest.extend_from_slice(&buf[..n]);
        }
        Ok((resp, None))
    }
}

/// Incremental `Transfer-Encoding: chunked` decoder.
#[derive(Default)]
struct ChunkDecoder {
    buf: Vec<u8>,
    finished: bool,
}

impl ChunkDecoder {
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.windows(2).position(|w| w == b"\r\n") {
            let size_text = String::from_utf8_lossy(&self.buf[..nl]).into_owned();
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
            if size == 0 {
                self.finished = true;
                self.buf.clear();
                break;
            }
            if self.buf.len() < nl + 2 + size + 2 {
                break;
            }
            out.extend_from_slice(&self.buf[nl + 2..nl + 2 + size]);
            self.buf.drain(..nl + 2 + size + 2);
        }
        out
    }
}

struct Parsed {
    tls: bool,
    host: String,
    host_header: String,
    port: u16,
    target: String,
}

fn parse_url(url: &str) -> Result<Parsed, URLError> {
    let bad = || URLError::BadURL(url.to_string());
    let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
    let tls = match scheme {
        "https" => true,
        "http" => false,
        _ => return Err(bad()),
    };
    let (auth, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let path = path.split('#').next().unwrap_or("/");
    let target = if path.starts_with('?') { format!("/{path}") } else { path.to_string() };
    let (host, port) = match auth.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| bad())?),
        None => (auth, if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(bad());
    }
    Ok(Parsed { tls, host: host.to_string(), host_header: auth.to_string(), port, target })
}

fn resolve(base: &str, loc: &str) -> Result<String, URLError> {
    if loc.contains("://") {
        return Ok(loc.to_string());
    }
    let p = parse_url(base)?;
    let origin = format!("{}://{}", if p.tls { "https" } else { "http" }, p.host_header);
    if loc.starts_with('/') {
        return Ok(format!("{origin}{loc}"));
    }
    let dir = p.target.split('?').next().unwrap_or("/");
    let dir = &dir[..dir.rfind('/').map_or(0, |i| i + 1)];
    Ok(format!("{origin}{dir}{loc}"))
}

/// Cookie hand-off for `URLSession` (`HTTPCookieStorage` parity): sets the `Cookie` header of `request` to the cookies
/// `storage` holds for its URL at `now` (Unix seconds). A request with no matching cookie is left unchanged.
/// The session does not call this itself yet: an app that keeps a jar applies it before `data_task`.
pub fn attach_cookies(request: &mut URLRequest, storage: &crate::http_cookies::HTTPCookieStorage, now: u64) {
    if let Some(header) = storage.cookie_header(&request.url, now) {
        request.set_header("Cookie", &header);
    }
}

/// Stores the `Set-Cookie` headers of `response`, received at `now`, in `storage`. Returns how many were accepted.
/// Pairs with `attach_cookies`: an app runs it on each reply it gets back.
pub fn store_cookies(response: &HTTPURLResponse, storage: &mut crate::http_cookies::HTTPCookieStorage, now: u64) -> usize {
    storage.set_cookies_from_headers(&response.url, &response.headers, now)
}

#[cfg(test)]
mod cookie_tests {
    use super::*;
    use crate::http_cookies::HTTPCookieStorage;

    fn reply(url: &str, set_cookies: &[&str]) -> HTTPURLResponse {
        HTTPURLResponse {
            url: url.to_string(),
            status_code: 200,
            headers: set_cookies.iter().map(|v| ("Set-Cookie".to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn a_stored_cookie_is_attached_to_a_matching_request_only() {
        let mut jar = HTTPCookieStorage::new();
        assert_eq!(store_cookies(&reply("https://shop.example/", &["sid=abc; Path=/"]), &mut jar, 100), 1);
        let mut same_site = URLRequest::new("https://shop.example/cart");
        attach_cookies(&mut same_site, &jar, 100);
        assert_eq!(same_site.headers, vec![("Cookie".to_string(), "sid=abc".to_string())]);
        let mut other = URLRequest::new("https://other.example/");
        attach_cookies(&mut other, &jar, 100);
        assert!(other.headers.is_empty());
    }

    #[test]
    fn secure_cookies_need_https() {
        let mut jar = HTTPCookieStorage::new();
        assert_eq!(store_cookies(&reply("https://shop.example/", &["token=t; Secure; Path=/"]), &mut jar, 100), 1);
        assert_eq!(jar.cookie_header("http://shop.example/", 100), None);
        assert_eq!(jar.cookie_header("https://shop.example/", 100), Some("token=t".to_string()));
    }

    #[test]
    fn max_age_expires_the_cookie_at_its_deadline() {
        let mut jar = HTTPCookieStorage::new();
        store_cookies(&reply("https://shop.example/", &["tmp=1; Max-Age=60; Path=/"]), &mut jar, 100);
        assert_eq!(jar.cookie_header("https://shop.example/", 159), Some("tmp=1".to_string()));
        assert_eq!(jar.cookie_header("https://shop.example/", 160), None);
    }

    #[test]
    fn only_set_cookie_headers_are_stored_and_longer_paths_go_first() {
        let mut jar = HTTPCookieStorage::new();
        let resp = HTTPURLResponse {
            url: "https://shop.example/".to_string(),
            status_code: 200,
            headers: vec![
                ("Set-Cookie".to_string(), "a=1; Path=/".to_string()),
                ("Content-Type".to_string(), "text/html".to_string()),
                ("set-cookie".to_string(), "b=2; Path=/cart".to_string()),
            ],
        };
        assert_eq!(store_cookies(&resp, &mut jar, 100), 2);
        assert_eq!(jar.cookies().len(), 2);
        assert_eq!(jar.cookie_header("https://shop.example/", 100), Some("a=1".to_string()));
        assert_eq!(jar.cookie_header("https://shop.example/cart", 100), Some("b=2; a=1".to_string()));
    }

    #[test]
    fn a_cookie_with_the_same_name_and_path_replaces_the_old_one() {
        let mut jar = HTTPCookieStorage::new();
        store_cookies(&reply("https://shop.example/", &["a=1; Path=/"]), &mut jar, 100);
        assert_eq!(store_cookies(&reply("https://shop.example/", &["a=2; Path=/"]), &mut jar, 101), 1);
        assert_eq!(jar.cookies().len(), 1);
        assert_eq!(jar.cookie_header("https://shop.example/", 101), Some("a=2".to_string()));
    }

    #[test]
    fn a_domain_attribute_for_another_host_is_refused() {
        let mut jar = HTTPCookieStorage::new();
        assert_eq!(store_cookies(&reply("https://shop.example/", &["x=1; Domain=other.example"]), &mut jar, 100), 0);
        assert!(jar.cookies().is_empty());
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;

    /// Answers every connection with `reply` and keeps the bytes the client wrote, so a test can read the request.
    struct FakeTransport {
        reply: Vec<u8>,
        written: Arc<Mutex<Vec<u8>>>,
        connects: Arc<AtomicUsize>,
    }

    struct FakeConn {
        reply: Cursor<Vec<u8>>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl Read for FakeConn {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reply.read(buf)
        }
    }

    impl Write for FakeConn {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Transport for FakeTransport {
        fn connect(&self, _host: &str, _port: u16, _tls: bool) -> std::io::Result<Box<dyn Stream>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeConn { reply: Cursor::new(self.reply.clone()), written: self.written.clone() }))
        }
    }

    struct Rig {
        session: URLSession,
        written: Arc<Mutex<Vec<u8>>>,
        connects: Arc<AtomicUsize>,
    }

    fn rig() -> Rig {
        let written = Arc::new(Mutex::new(Vec::new()));
        let connects = Arc::new(AtomicUsize::new(0));
        let transport = FakeTransport {
            reply: b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
            written: written.clone(),
            connects: connects.clone(),
        };
        Rig { session: URLSession::with_transport(Arc::new(transport), None), written, connects }
    }

    fn upload_request() -> URLRequest {
        let mut req = URLRequest::new("http://upload.test/inbox");
        req.method = "POST".into();
        req
    }

    /// The request head as text, and the bytes after its blank line.
    fn split_request(raw: &[u8]) -> (String, Vec<u8>) {
        let at = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a blank line ends the head");
        // the head keeps its last header's line ending
        (String::from_utf8_lossy(&raw[..at + 2]).into_owned(), raw[at + 4..].to_vec())
    }

    fn temp_file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn a_new_task_reports_no_expected_length_yet() {
        assert_eq!(URLSessionTask::default().count_of_bytes_expected_to_send(), TRANSFER_SIZE_UNKNOWN);
        assert_eq!(URLSessionTask::default().count_of_bytes_sent(), 0);
    }

    #[test]
    fn a_data_upload_sends_the_body_with_content_length_and_reports_progress() {
        let r = rig();
        let task = URLSessionTask::default();
        let mut progress = Vec::new();
        let (resp, body) = r
            .session
            .upload_with(&upload_request(), || Ok(Payload::Bytes(b"hello".to_vec())), &task, &mut |c, sent, total| progress.push((c, sent, total)))
            .unwrap();
        assert_eq!((resp.status_code, body.as_slice()), (200, &b"ok"[..]));
        let (head, sent_body) = split_request(&r.written.lock().unwrap());
        assert!(head.starts_with("POST /inbox HTTP/1.1\r\n"));
        assert!(head.contains("Content-Length: 5\r\n"));
        assert_eq!(sent_body, b"hello");
        assert_eq!(progress, vec![(5, 5, 5)]);
        assert_eq!((task.count_of_bytes_sent(), task.count_of_bytes_expected_to_send()), (5, 5));
    }

    #[test]
    fn a_file_upload_streams_the_file_with_monotonic_progress_ending_at_total() {
        let r = rig();
        let bytes: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        let (_dir, path) = temp_file(&bytes);
        let task = URLSessionTask::default();
        let mut progress = Vec::new();
        r.session.upload_with(&upload_request(), || Payload::open(&path), &task, &mut |c, sent, total| progress.push((c, sent, total))).unwrap();
        let (head, body) = split_request(&r.written.lock().unwrap());
        assert!(head.contains("Content-Length: 150000\r\n"));
        assert_eq!(body, bytes);
        assert!(progress.len() > 1, "a 150 kB file goes out in several chunks");
        assert!(progress.windows(2).all(|w| w[0].1 <= w[1].1), "the sent total never goes down");
        assert!(progress.iter().all(|&(_, _, total)| total == 150_000));
        assert_eq!(progress.iter().map(|p| p.0).sum::<u64>(), 150_000);
        assert_eq!(progress.last().unwrap().1, 150_000);
        assert_eq!(task.count_of_bytes_sent(), 150_000);
    }

    #[test]
    fn cancel_before_the_upload_starts_yields_cancelled_and_sends_nothing() {
        let r = rig();
        let task = URLSessionTask::default();
        task.cancel();
        let res = r.session.upload_with(&upload_request(), || Ok(Payload::Bytes(b"x".to_vec())), &task, &mut |_, _, _| {});
        assert_eq!(res.unwrap_err(), URLError::Cancelled);
        assert_eq!(r.connects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancel_during_a_file_upload_stops_between_chunks() {
        let r = rig();
        let (_dir, path) = temp_file(&vec![1u8; 300_000]);
        let task = URLSessionTask::default();
        let stopper = task.clone();
        let res = r.session.upload_with(&upload_request(), || Payload::open(&path), &task, &mut |_, sent, _| {
            if sent >= UPLOAD_CHUNK as u64 {
                stopper.cancel();
            }
        });
        assert_eq!(res.unwrap_err(), URLError::Cancelled);
        assert!(task.count_of_bytes_sent() < 300_000);
    }

    #[test]
    fn a_missing_file_fails_before_any_request_is_made() {
        let r = rig();
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.bin");
        let res = r.session.upload_with(&upload_request(), || Payload::open(&missing), &URLSessionTask::default(), &mut |_, _, _| {});
        assert!(matches!(res, Err(URLError::FileError(_))));
        assert_eq!(r.connects.load(Ordering::SeqCst), 0);
    }
}
