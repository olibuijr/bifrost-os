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
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Handle for cancelling a task (`URLSessionTask.cancel()`).
#[derive(Clone, Default)]
pub struct URLSessionTask {
    cancel: Arc<AtomicBool>,
}

impl URLSessionTask {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
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

    /// `URLSession.bytes(for:)`: blocking, call from a background thread. `on_response` runs once with the final head;
    /// `on_chunk` gets decoded body bytes (any status) and returns false to stop (result `Cancelled`).
    pub fn bytes(
        &self,
        req: &URLRequest,
        cancel: &AtomicBool,
        on_response: &mut dyn FnMut(&HTTPURLResponse),
        on_chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<HTTPURLResponse, URLError> {
        self.fetch(req, cancel, on_response, on_chunk)
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
        let resp = self.fetch(&req, cancel, &mut |_| {}, &mut |c| {
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
        let result = self.fetch(
            &req,
            cancel,
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

    /// One request with redirects. Redirect bodies are skipped; `on_response`/`on_chunk` see the final reply.
    fn fetch(&self, req: &URLRequest, cancel: &AtomicBool, on_response: &mut dyn FnMut(&HTTPURLResponse), on_chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<HTTPURLResponse, URLError> {
        let deadline = Instant::now() + req.timeout;
        let (mut url, mut method, mut body) = (req.url.clone(), req.method.clone(), req.body.clone());
        for _ in 0..=MAX_REDIRECTS {
            let (resp, redirect) = self.once(&url, &method, &req.headers, body.as_deref(), cancel, deadline, on_response, on_chunk)?;
            match redirect {
                Some(loc) => {
                    url = resolve(&url, &loc)?;
                    if resp.status_code == 303 || (resp.status_code != 307 && resp.status_code != 308 && method == "POST") {
                        method = "GET".into();
                        body = None;
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
        body: Option<&[u8]>,
        cancel: &AtomicBool,
        deadline: Instant,
        on_response: &mut dyn FnMut(&HTTPURLResponse),
        on_chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(HTTPURLResponse, Option<String>), URLError> {
        let p = parse_url(url)?;
        let mut conn = self.transport.connect(&p.host, p.port, p.tls).map_err(|e| URLError::CannotConnect(format!("{}: {e}", p.host)))?;
        let mut head = format!("{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: BifrostApp/1\r\nAccept: */*\r\nConnection: close\r\n", p.target, p.host_header);
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(b) = body {
            head.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        head.push_str("\r\n");
        conn.write_all(head.as_bytes()).and_then(|_| conn.write_all(body.unwrap_or(&[]))).and_then(|_| conn.flush()).map_err(|e| URLError::CannotConnect(e.to_string()))?;

        let check = || -> Result<(), URLError> {
            if cancel.load(Ordering::Relaxed) {
                Err(URLError::Cancelled)
            } else if Instant::now() > deadline {
                Err(URLError::TimedOut)
            } else {
                Ok(())
            }
        };
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
