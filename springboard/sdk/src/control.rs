//! App control endpoint (tasks/app-control.md): one JSON-RPC 2.0 request per
//! connection, newline-terminated, on an owner-only Unix socket. The SDK
//! serves it from the app's main loop, so handlers run on the thread that owns
//! the app state. Only the owning UID may connect (SO_PEERCRED); root too when the app runs outside a user namespace
//! (root can do anything to it there anyway). The shell dials sandboxed apps with the account's credentials.

use bifrost_core::json::{self, Json};
use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Largest request accepted, in bytes (including the newline).
pub const MAX_REQUEST: usize = 256 * 1024;
/// Connections served per wake-up, so a flood cannot starve the UI.
const MAX_PER_WAKE: usize = 8;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// Server error: an async call ran past its `timeout_ms`.
pub const TIMEOUT: i64 = -32000;
/// Not sent on the wire: an `App::rpc` handler returns `RpcError::pending(token, timeout_ms)` to answer later.
pub const PENDING: i64 = -32099;
/// Longest wait an async call may ask for, milliseconds.
pub const MAX_TIMEOUT_MS: u64 = 60_000;
/// Async calls held open at once; more are refused with `INTERNAL_ERROR`.
const MAX_WAITING: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    /// Sent as `error.data.reason` when set.
    pub reason: Option<String>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), reason: None }
    }

    /// Adds `error.data.reason` to the reply.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// "Answer later": the connection stays open until `complete(token, ..)` is called, or `timeout_ms` (default
    /// 10000 when 0, at most `MAX_TIMEOUT_MS`) passes and the caller gets a `TIMEOUT` error.
    pub fn pending(token: u64, timeout_ms: u64) -> Self {
        let t = if timeout_ms == 0 { 10_000 } else { timeout_ms.min(MAX_TIMEOUT_MS) };
        RpcError { code: PENDING, message: format!("{token}:{t}"), reason: None }
    }

    /// `(token, timeout_ms)` when this is a `pending` marker.
    pub fn pending_token(&self) -> Option<(u64, u64)> {
        if self.code != PENDING {
            return None;
        }
        let (a, b) = self.message.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    }
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
type Completion = (u64, Result<Json, RpcError>);
/// Most unclaimed completions kept; older ones are dropped.
const MAX_COMPLETED: usize = 256;
static COMPLETED: Mutex<Vec<Completion>> = Mutex::new(Vec::new());

/// A fresh token for an async call.
pub fn new_token() -> u64 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// Finishes the async call `token`: the next `Control::service` pass writes the reply and closes the connection.
/// Safe to call from any thread; an unknown or expired token is ignored.
pub fn complete(token: u64, result: Result<Json, RpcError>) {
    if let Ok(mut q) = COMPLETED.lock() {
        if q.len() >= MAX_COMPLETED {
            // bounded: the oldest unclaimed answer belongs to a call that already timed out, so its result is dropped
            let _expired = q.remove(0);
        }
        q.push((token, result));
    }
}

/// An async call whose connection is held open.
struct Waiting {
    token: u64,
    id: Json,
    stream: UnixStream,
    deadline: Instant,
}

/// What one request line became.
pub enum Line {
    /// the reply text (no newline)
    Done(String),
    /// the handler asked to answer later
    Pending { id: Json, token: u64, timeout_ms: u64 },
}

pub struct Control {
    listener: UnixListener,
    path: PathBuf,
    waiting: Vec<Waiting>,
}

impl Control {
    /// Binds `path` (creating its directory 0700 and the socket 0600). A stale socket file is replaced.
    pub fn bind(path: &Path) -> io::Result<Control> {
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Control { listener, path: path.to_path_buf(), waiting: Vec::new() })
    }

    /// The descriptor to poll for incoming connections.
    pub fn fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True while async calls are waiting: the app loop should call `service` at least every 100 ms then.
    pub fn has_pending(&self) -> bool {
        !self.waiting.is_empty() || COMPLETED.lock().map(|q| !q.is_empty()).unwrap_or(false)
    }

    /// Serves the connections waiting now. `handle(method, params)` answers each request. Also writes the replies of
    /// async calls that were completed and times out the ones past their deadline.
    pub fn service(&mut self, handle: &mut dyn FnMut(&str, &Json) -> Result<Json, RpcError>) {
        for _ in 0..MAX_PER_WAKE {
            let stream = match self.listener.accept() {
                Ok((s, _)) => s,
                Err(_) => break,
            };
            if !peer_is_owner(&stream) {
                continue; // dropped: another user
            }
            if let Some((stream, id, token, timeout_ms)) = serve(stream, handle) {
                if self.waiting.len() >= MAX_WAITING {
                    reply_to(stream, error_reply(&id, INTERNAL_ERROR, "too many pending calls"));
                } else {
                    self.waiting.push(Waiting { token, id, stream, deadline: Instant::now() + Duration::from_millis(timeout_ms) });
                }
            }
        }
        self.finish_async();
    }

    fn finish_async(&mut self) {
        let done: Vec<Completion> = match COMPLETED.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => Vec::new(),
        };
        for (token, result) in done {
            if let Some(i) = self.waiting.iter().position(|w| w.token == token) {
                let w = self.waiting.remove(i);
                let text = match result {
                    Ok(v) => ok_reply(&w.id, &v),
                    Err(e) => error_reply_e(&w.id, &e),
                };
                reply_to(w.stream, text);
            }
        }
        let now = Instant::now();
        let mut i = 0;
        while i < self.waiting.len() {
            if self.waiting[i].deadline <= now {
                let w = self.waiting.remove(i);
                reply_to(w.stream, error_reply(&w.id, TIMEOUT, "timeout"));
            } else {
                i += 1;
            }
        }
    }
}

/// Writes an async reply without ever blocking the UI thread: a short nonblocking attempt first, and a writer
/// thread (5 s write timeout) only when the client's socket buffer is full.
fn reply_to(mut stream: UnixStream, mut text: String) {
    text.push('\n');
    let _ = stream.set_nonblocking(true);
    let bytes = text.into_bytes();
    let mut sent = 0;
    while sent < bytes.len() {
        match stream.write(&bytes[sent..]) {
            Ok(0) => return,
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let rest = bytes[sent..].to_vec();
                let _ = std::thread::Builder::new().name("control-reply".into()).spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.write_all(&rest);
                });
                return;
            }
            Err(_) => return,
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Reads one request line, answers it, closes. Bounded by timeouts so a silent client cannot stall the app.
/// Returns the stream back when the handler asked to answer later.
fn serve(mut stream: UnixStream, handle: &mut dyn FnMut(&str, &Json) -> Result<Json, RpcError>) -> Option<(UnixStream, Json, u64, u64)> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let line = loop {
        if let Some(i) = buf.iter().position(|&b| b == b'\n') {
            break String::from_utf8_lossy(&buf[..i]).into_owned();
        }
        if buf.len() >= MAX_REQUEST {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break String::from_utf8_lossy(&buf).into_owned(),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    };
    match handle_line_ex(&line, handle) {
        Line::Done(reply) => {
            reply_to(stream, reply);
            None
        }
        Line::Pending { id, token, timeout_ms } => Some((stream, id, token, timeout_ms)),
    }
}

/// Parses one request and returns the JSON-RPC response text. A handler's `pending` marker becomes an error here;
/// `Control::service` is the async-capable path.
pub fn handle_line(line: &str, handle: &mut dyn FnMut(&str, &Json) -> Result<Json, RpcError>) -> String {
    match handle_line_ex(line, handle) {
        Line::Done(t) => t,
        Line::Pending { id, .. } => error_reply(&id, INTERNAL_ERROR, "async reply not supported here"),
    }
}

/// Like `handle_line`, but keeps a `pending` marker so the caller can hold the connection.
pub fn handle_line_ex(line: &str, handle: &mut dyn FnMut(&str, &Json) -> Result<Json, RpcError>) -> Line {
    let req = match json::parse(line) {
        Ok(v) => v,
        Err(e) => return Line::Done(error_reply(&Json::Null, PARSE_ERROR, &format!("parse error: {e}"))),
    };
    let id = req.get("id").cloned().unwrap_or(Json::Null);
    if req.get("jsonrpc").and_then(Json::as_str) != Some("2.0") {
        return Line::Done(error_reply(&id, INVALID_REQUEST, "jsonrpc must be \"2.0\""));
    }
    let Some(method) = req.get("method").and_then(Json::as_str) else {
        return Line::Done(error_reply(&id, INVALID_REQUEST, "method must be a string"));
    };
    let params = req.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
    match handle(method, &params) {
        Ok(result) => Line::Done(ok_reply(&id, &result)),
        Err(e) => match e.pending_token() {
            Some((token, timeout_ms)) => Line::Pending { id, token, timeout_ms },
            None => Line::Done(error_reply_e(&id, &e)),
        },
    }
}

fn ok_reply(id: &Json, result: &Json) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{}}}", encode(id), encode(result))
}

fn error_reply_e(id: &Json, e: &RpcError) -> String {
    match &e.reason {
        None => error_reply(id, e.code, &e.message),
        Some(r) => format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{},\"message\":{},\"data\":{{\"reason\":{}}}}}}}",
            encode(id),
            e.code,
            json::quote(&e.message),
            json::quote(r)
        ),
    }
}

fn error_reply(id: &Json, code: i64, message: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{code},\"message\":{}}}}}",
        encode(id),
        json::quote(message)
    )
}

/// Compact JSON text for a value (`Json::encode`).
pub fn encode(v: &Json) -> String {
    v.encode()
}

#[repr(C)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

extern "C" {
    fn getsockopt(fd: i32, level: i32, name: i32, val: *mut c_void, len: *mut u32) -> i32;
    fn geteuid() -> u32;
}

const SOL_SOCKET: i32 = 1;
const SO_PEERCRED: i32 = 17;

/// True when the connecting process runs as this app's UID, or as root while the app runs in the initial user
/// namespace (an app started outside the sandbox). A sandboxed app's user namespace maps only the account's UID, so
/// the shell dials it with the account's credentials (`shell::intents`); a host root there shows up as the overflow
/// UID and is refused, as is every other user.
fn peer_is_owner(stream: &UnixStream) -> bool {
    let mut cred = Ucred { pid: 0, uid: u32::MAX, gid: 0 };
    let mut len = std::mem::size_of::<Ucred>() as u32;
    let rc = unsafe {
        getsockopt(stream.as_raw_fd(), SOL_SOCKET, SO_PEERCRED, &mut cred as *mut Ucred as *mut c_void, &mut len)
    };
    rc == 0 && peer_uid_allowed(cred.uid, unsafe { geteuid() }, in_initial_user_namespace())
}

/// Whether this process runs in the initial user namespace. A process's user namespace and its uid map are fixed
/// once the app runs, so `/proc/self/uid_map` is read once, on the first connection.
fn in_initial_user_namespace() -> bool {
    static INITIAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INITIAL.get_or_init(|| std::fs::read_to_string("/proc/self/uid_map").is_ok_and(|m| is_initial_uid_map(&m)))
}

/// True for the initial user namespace's uid map: the whole range mapped onto itself (`0 0 4294967295`).
fn is_initial_uid_map(map: &str) -> bool {
    let rows: Vec<Vec<&str>> = map.lines().map(|l| l.split_whitespace().collect::<Vec<&str>>()).filter(|r| !r.is_empty()).collect();
    rows.len() == 1 && rows[0] == ["0", "0", "4294967295"]
}

/// The peer rule: the app's own UID; root only when the app is in the initial user namespace (`initial_ns`), where
/// uid 0 is the real root. Inside a user namespace uid 0 would be a namespace-local root, never the shell.
fn peer_uid_allowed(peer: u32, own: u32, initial_ns: bool) -> bool {
    peer == own || (peer == 0 && initial_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            "echo" => Ok(params.clone()),
            "fail" => Err(RpcError::new(INVALID_PARAMS, "bad")),
            _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("no {method}"))),
        }
    }

    #[test]
    fn admits_the_owner_and_root_only() {
        assert!(peer_uid_allowed(1000, 1000, true));
        assert!(peer_uid_allowed(0, 1000, true), "an unsandboxed app admits the real root");
        assert!(peer_uid_allowed(0, 0, true));
        assert!(!peer_uid_allowed(1001, 1000, true), "a foreign uid is refused");
        assert!(!peer_uid_allowed(u32::MAX, 1000, true), "a failed lookup is refused");
        assert!(!peer_uid_allowed(1000, 0, true), "a root app admits only root");
    }

    #[test]
    fn a_sandboxed_app_admits_only_its_own_uid() {
        // in the app's user namespace (`1000 1000 1`) the host root arrives as the overflow uid, and a uid 0 would be
        // a namespace-local root: neither is the shell, which dials as the account
        assert!(peer_uid_allowed(1000, 1000, false));
        assert!(!peer_uid_allowed(0, 1000, false));
        assert!(!peer_uid_allowed(65534, 1000, false));
    }

    #[test]
    fn only_the_identity_uid_map_is_the_initial_namespace() {
        assert!(is_initial_uid_map("         0          0 4294967295\n"));
        assert!(!is_initial_uid_map("      1000       1000          1\n"));
        assert!(!is_initial_uid_map("0 100000 65536\n"));
        assert!(!is_initial_uid_map("0 0 4294967295\n1 1 1\n"));
        assert!(!is_initial_uid_map(""));
    }

    #[test]
    fn answers_a_call_with_the_same_id() {
        let out = handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"echo","params":{"a":[1,"x"]}}"#, &mut echo);
        assert_eq!(out, r#"{"jsonrpc":"2.0","id":7,"result":{"a":[1,"x"]}}"#);
    }

    #[test]
    fn maps_errors_to_json_rpc_codes() {
        let out = handle_line(r#"{"jsonrpc":"2.0","id":"q","method":"fail"}"#, &mut echo);
        assert_eq!(out, r#"{"jsonrpc":"2.0","id":"q","error":{"code":-32602,"message":"bad"}}"#);
        let out = handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#, &mut echo);
        assert!(out.contains("-32601"));
    }

    #[test]
    fn rejects_malformed_requests() {
        assert!(handle_line("{", &mut echo).contains("-32700"));
        assert!(handle_line(r#"{"id":1,"method":"echo"}"#, &mut echo).contains("-32600"));
        assert!(handle_line(r#"{"jsonrpc":"2.0","id":1}"#, &mut echo).contains("-32600"));
    }

    #[test]
    fn pending_marker_round_trips_and_clamps() {
        let e = RpcError::pending(7, 0);
        assert_eq!(e.pending_token(), Some((7, 10_000)));
        assert_eq!(RpcError::pending(1, 999_999).pending_token(), Some((1, MAX_TIMEOUT_MS)));
        assert_eq!(RpcError::new(INVALID_PARAMS, "x").pending_token(), None);
    }

    #[test]
    fn async_call_is_completed_or_times_out() {
        let dir = std::env::temp_dir().join(format!("sdk-control-async-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("app.sock");
        let mut ctl = Control::bind(&path).expect("bind");
        let call = |p: PathBuf, method: &'static str| {
            std::thread::spawn(move || {
                let mut s = UnixStream::connect(p).unwrap();
                s.write_all(format!("{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"{method}\"}}\n").as_bytes()).unwrap();
                let mut out = String::new();
                s.read_to_string(&mut out).unwrap();
                out
            })
        };
        let slow = call(path.clone(), "later");
        let lost = call(path.clone(), "never");
        let mut tokens: Vec<(String, u64)> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(slow.is_finished() && lost.is_finished()) && Instant::now() < deadline {
            ctl.service(&mut |m, _| {
                let t = new_token();
                tokens.push((m.to_string(), t));
                Err(RpcError::pending(t, if m == "never" { 150 } else { 5000 }))
            });
            if let Some((_, t)) = tokens.iter().find(|(m, _)| m == "later") {
                complete(*t, Ok(Json::Bool(true)));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(slow.join().unwrap(), "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":true}\n");
        assert!(lost.join().unwrap().contains("-32000"));
        drop(ctl);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completed_map_is_bounded() {
        for t in 0..(MAX_COMPLETED as u64 + 50) {
            complete(9_000_000 + t, Ok(Json::Null));
        }
        assert!(COMPLETED.lock().unwrap().len() <= MAX_COMPLETED);
    }

    #[test]
    fn encodes_numbers_and_escapes() {
        assert_eq!(encode(&Json::Num(3.0)), "3");
        assert_eq!(encode(&Json::Num(2.5)), "2.5");
        assert_eq!(encode(&Json::Str("a\"b\n".into())), r#""a\"b\n""#);
    }

    #[test]
    fn socket_is_owner_only_and_serves_one_request() {
        let dir = std::env::temp_dir().join(format!("sdk-control-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("app.sock");
        let mut ctl = Control::bind(&path).expect("bind");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);

        let client = std::thread::spawn({
            let path = path.clone();
            move || {
                let mut s = UnixStream::connect(path).unwrap();
                s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"echo\",\"params\":[]}\n").unwrap();
                let mut out = String::new();
                s.read_to_string(&mut out).unwrap();
                out
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !client.is_finished() && std::time::Instant::now() < deadline {
            ctl.service(&mut echo);
            std::thread::sleep(Duration::from_millis(5));
        }
        let out = client.join().unwrap();
        assert_eq!(out, "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[]}\n");
        drop(ctl);
        assert!(!path.exists(), "socket removed on drop");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
