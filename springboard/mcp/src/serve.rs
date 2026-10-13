//! `bifrost-mcp --serve`: the system service (bifrost-mcp.service, on by default). Agents connect over
//! - MCP Streamable HTTP at http://<machine>:7420/mcp (POST JSON-RPC, JSON replies, keep-alive), with
//!   `Authorization: Bearer <token>` from /var/lib/bifrost-mcp/token (made on first start; required on every TCP request, local agents use the unix socket);
//! - a Unix socket, /run/bifrost-mcp.sock (newline-delimited JSON-RPC, like stdio), for agents on the machine:
//!   root and the desktop account's group.
//! All sessions share one context behind a lock, so input from several agents is applied in order.
//! Input is bounded before it is buffered: lines, headers, bodies and open HTTP connections all have caps.

use crate::tools::Ctx;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TOKEN_PATH: &str = "/var/lib/bifrost-mcp/token";
pub const SOCKET: &str = "/run/bifrost-mcp.sock";
const MAX_BODY: usize = 8 << 20;
/// Longest request or header line, newline included. Longer input is refused before it is buffered.
const MAX_LINE: usize = 8 << 10;
/// Total bytes of header lines in one request.
const HEADER_LIMIT: usize = 16 << 10;
const MAX_HEADERS: usize = 100;
/// Open HTTP connections; more are answered 503.
const MAX_HTTP_SESSIONS: usize = 32;
/// Deadline for the whole request head of a connection that has not presented a token yet.
const PRE_AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Read timeout for a connection that has authenticated (keep-alive agents).
const SESSION_TIMEOUT: Duration = Duration::from_secs(300);

type Shared = Arc<Mutex<Ctx>>;

static OPEN_HTTP: AtomicUsize = AtomicUsize::new(0);

/// One of the MAX_HTTP_SESSIONS connection slots, released when its session ends.
struct Slot;

impl Slot {
    fn acquire() -> Option<Slot> {
        if OPEN_HTTP.fetch_add(1, Ordering::SeqCst) < MAX_HTTP_SESSIONS {
            Some(Slot)
        } else {
            OPEN_HTTP.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }
}

/// A TCP stream whose reads share one deadline: each read waits only for the time left, so a slow peer cannot stretch a request.
struct Timed {
    s: TcpStream,
    deadline: Option<Instant>,
}

impl Read for Timed {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "request deadline passed"));
            }
            self.s.set_read_timeout(Some(left))?;
        }
        self.s.read(b)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        OPEN_HTTP.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn run(ctx: Ctx) {
    let ctx: Shared = Arc::new(Mutex::new(ctx));
    let token = Arc::new(token());
    let port: u16 = std::env::var("BIFROST_MCP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7420);
    let addr = std::env::var("BIFROST_MCP_LISTEN").unwrap_or_else(|_| "0.0.0.0".into());
    let _ = std::fs::remove_file(SOCKET);
    match UnixListener::bind(SOCKET) {
        Ok(l) => {
            let _ = std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o660));
            if let Some((_, _, gid)) = crate::sys::account() {
                let _ = std::os::unix::fs::chown(SOCKET, Some(0), Some(gid));
            }
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                for s in l.incoming().flatten() {
                    let ctx = ctx.clone();
                    std::thread::spawn(move || unix_session(s, ctx));
                }
            });
        }
        Err(e) => eprintln!("bifrost-mcp: {SOCKET}: {e}"),
    }
    let l = TcpListener::bind((addr.as_str(), port)).unwrap_or_else(|e| {
        eprintln!("bifrost-mcp: {addr}:{port}: {e}");
        std::process::exit(1)
    });
    eprintln!("bifrost-mcp {} serving http://{addr}:{port}/mcp and {SOCKET}", env!("CARGO_PKG_VERSION"));
    for s in l.incoming().flatten() {
        match Slot::acquire() {
            Some(slot) => {
                let (ctx, token) = (ctx.clone(), token.clone());
                std::thread::spawn(move || {
                    let _slot = slot;
                    http_session(s, ctx, &token)
                });
            }
            None => {
                let mut s = s;
                // a peer that never reads must not stall the accept loop
                let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = respond(&mut s, 503, "text/plain", b"too many sessions", true);
            }
        }
    }
}

/// The bearer token, created (random, 0600) on first start. Exits rather than run with a guessable token.
fn token() -> String {
    if let Ok(t) = std::fs::read_to_string(TOKEN_PATH) {
        if t.trim().len() >= 32 {
            let _ = std::fs::set_permissions(TOKEN_PATH, std::fs::Permissions::from_mode(0o600));
            return t.trim().to_string();
        }
    }
    if std::path::Path::new(TOKEN_PATH).exists() {
        eprintln!("bifrost-mcp: {TOKEN_PATH} was unreadable or too short; making a new token");
    }
    let t = make_token(fill_random).unwrap_or_else(|e| fatal(&format!("no entropy for the token: {e}")));
    write_token(&t).unwrap_or_else(|e| fatal(&format!("{TOKEN_PATH}: {e}")));
    t
}

fn fatal(msg: &str) -> ! {
    eprintln!("bifrost-mcp: {msg}; not starting");
    std::process::exit(1)
}

/// Fills `b` from the kernel's entropy source.
fn fill_random(b: &mut [u8]) -> io::Result<()> {
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(b))
}

/// A 24-byte token as 48 hex digits. A failing entropy source is an error, never an all-zero token.
fn make_token(fill: fn(&mut [u8]) -> io::Result<()>) -> io::Result<String> {
    let mut b = [0u8; 24];
    fill(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Creates the token file with mode 0600 from its first byte, so no other user can read it.
fn write_token(t: &str) -> io::Result<()> {
    std::fs::create_dir_all("/var/lib/bifrost-mcp")?;
    // a short or unreadable leftover; the new file must be created, never reused
    let _ = std::fs::remove_file(TOKEN_PATH);
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(TOKEN_PATH)?;
    writeln!(f, "{t}")
}

/// Reads one line of at most `max` bytes, newline included, and returns "" at end of input. A line that
/// reaches `max` bytes without a newline is an error, so a peer cannot make the server buffer without limit.
fn read_bounded_line<R: BufRead>(r: &mut R, max: usize) -> io::Result<String> {
    let mut buf = Vec::new();
    r.by_ref().take(max as u64).read_until(b'\n', &mut buf)?;
    if buf.len() == max && buf.last() != Some(&b'\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    String::from_utf8(buf).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "line is not UTF-8"))
}

#[derive(Debug)]
struct Head {
    method: String,
    path: String,
    len: usize,
    auth: String,
    close: bool,
}

/// Reads a request line and its headers. Ok(None) when the peer closed before a request line.
fn parse_head<R: BufRead>(r: &mut R) -> io::Result<Option<Head>> {
    let line = read_bounded_line(r, MAX_LINE)?;
    if line.is_empty() {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let (mut len, mut auth, mut close) = (0usize, String::new(), false);
    let (mut total, mut count, mut seen_len) = (0usize, 0usize, false);
    loop {
        let raw = read_bounded_line(r, MAX_LINE)?;
        if raw.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "headers cut short"));
        }
        total += raw.len();
        if total > HEADER_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "headers too large"));
        }
        let h = raw.trim_end();
        if h.is_empty() {
            break;
        }
        count += 1;
        if count > MAX_HEADERS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many headers"));
        }
        let (k, v) = h.split_once(':').unwrap_or((h, ""));
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                // a bad or repeated length would leave body bytes to be read as the next request
                if seen_len {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "two content-length headers"));
                }
                seen_len = true;
                len = v.trim().parse::<usize>().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad content-length"))?;
            }
            "authorization" => auth = v.trim().to_string(),
            "connection" => close = v.trim().eq_ignore_ascii_case("close"),
            _ => {}
        }
    }
    Ok(Some(Head { method, path, len, auth, close }))
}

/// True when `auth` is `Bearer <token>`. The comparison touches every byte, so timing does not reveal the token.
fn bearer_ok(auth: &str, token: &str) -> bool {
    let Some(got) = auth.strip_prefix("Bearer ").map(str::trim) else {
        return false;
    };
    got.len() == token.len() && got.bytes().zip(token.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

fn unix_session(s: UnixStream, ctx: Shared) {
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut r = BufReader::new(s);
    loop {
        let line = match read_bounded_line(&mut r, MAX_BODY) {
            Ok(l) if !l.is_empty() => l,
            _ => break,
        };
        let line = line.trim_end();
        let reply = crate::handle(&mut ctx.lock().unwrap_or_else(|p| p.into_inner()), line);
        if let Some(r) = reply {
            if writeln!(w, "{r}").is_err() {
                break;
            }
        }
    }
}

fn http_session(s: TcpStream, ctx: Shared, token: &str) {
    let _ = s.set_nodelay(true);
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut r = BufReader::new(Timed { s, deadline: None });
    let mut authed = false;
    loop {
        // a connection that has not presented a token yet gets one deadline for its request head
        if authed {
            r.get_mut().deadline = None;
            let _ = r.get_ref().s.set_read_timeout(Some(SESSION_TIMEOUT));
        } else {
            r.get_mut().deadline = Some(Instant::now() + PRE_AUTH_TIMEOUT);
        }
        let head = match parse_head(&mut r) {
            Ok(Some(h)) => h,
            Ok(None) => return,
            Err(_) => {
                let _ = respond(&mut w, 400, "text/plain", b"bad request", true);
                return;
            }
        };
        if !head.path.starts_with("/mcp") {
            let _ = respond(&mut w, 404, "text/plain", b"MCP endpoint is /mcp", true);
            return;
        }
        // checked on every request, before any of its body is read
        if !bearer_ok(&head.auth, token) {
            let _ = respond(&mut w, 401, "application/json", br#"{"error":"bearer token required (/var/lib/bifrost-mcp/token)"}"#, true);
            return;
        }
        authed = true;
        // the pre-auth deadline covers the request head only; the body gets the session timeout
        r.get_mut().deadline = None;
        let _ = r.get_ref().s.set_read_timeout(Some(SESSION_TIMEOUT));
        if head.len > MAX_BODY {
            let _ = respond(&mut w, 413, "text/plain", b"too large", true);
            return;
        }
        let mut body = vec![0u8; head.len];
        if r.read_exact(&mut body).is_err() {
            return;
        }
        let ok = if head.method == "POST" {
            let text = String::from_utf8_lossy(&body);
            let reply = {
                let mut c = ctx.lock().unwrap_or_else(|p| p.into_inner());
                // a JSON-RPC batch is an array of messages
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Array(v)) => {
                        let out: Vec<_> = v.iter().filter_map(|m| crate::handle(&mut c, &m.to_string())).collect();
                        (!out.is_empty()).then_some(serde_json::Value::Array(out))
                    }
                    _ => crate::handle(&mut c, &text),
                }
            };
            match reply {
                Some(v) => respond(&mut w, 200, "application/json", v.to_string().as_bytes(), head.close),
                None => respond(&mut w, 202, "text/plain", b"", head.close),
            }
        } else if head.method == "DELETE" {
            respond(&mut w, 200, "text/plain", b"", head.close)
        } else {
            // no server-initiated stream: GET is not offered
            respond(&mut w, 405, "text/plain", b"POST JSON-RPC to /mcp", head.close)
        };
        if !ok || head.close {
            return;
        }
    }
}

fn respond(w: &mut TcpStream, code: u16, ctype: &str, body: &[u8], close: bool) -> bool {
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nMcp-Session-Id: bifrost\r\nConnection: {}\r\n\r\n",
        body.len(),
        if close { "close" } else { "keep-alive" }
    );
    w.write_all(head.as_bytes()).and_then(|_| w.write_all(body)).and_then(|_| w.flush()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn no_entropy(_: &mut [u8]) -> io::Result<()> {
        Err(io::Error::other("no urandom"))
    }

    fn fixed_entropy(b: &mut [u8]) -> io::Result<()> {
        b.fill(7);
        Ok(())
    }

    #[test]
    fn line_past_the_cap_without_newline_is_refused() {
        let long = vec![b'a'; 70 << 10];
        assert!(read_bounded_line(&mut Cursor::new(long), MAX_LINE).is_err());
    }

    #[test]
    fn normal_lines_are_returned_whole() {
        let mut c = Cursor::new(b"GET /mcp HTTP/1.1\r\nHost: x\r\n".to_vec());
        assert_eq!(read_bounded_line(&mut c, MAX_LINE).unwrap(), "GET /mcp HTTP/1.1\r\n");
        assert_eq!(read_bounded_line(&mut c, MAX_LINE).unwrap(), "Host: x\r\n");
        assert_eq!(read_bounded_line(&mut c, MAX_LINE).unwrap(), "");
    }

    #[test]
    fn head_with_more_than_100_headers_is_refused() {
        let mut text = String::from("POST /mcp HTTP/1.1\r\n");
        for i in 0..=MAX_HEADERS {
            text += &format!("X-H{i}: v\r\n");
        }
        text += "\r\n";
        assert!(parse_head(&mut Cursor::new(text)).is_err());
    }

    #[test]
    fn head_with_more_than_16_kib_of_headers_is_refused() {
        let pad = "a".repeat(6 << 10);
        let mut text = String::from("POST /mcp HTTP/1.1\r\n");
        for i in 0..3 {
            text += &format!("X-Pad{i}: {pad}\r\n");
        }
        text += "\r\n";
        assert!(parse_head(&mut Cursor::new(text)).is_err());
    }

    #[test]
    fn normal_head_is_parsed() {
        let text = "POST /mcp HTTP/1.1\r\nContent-Length: 12\r\nAuthorization: Bearer abc\r\nConnection: close\r\n\r\n";
        let h = parse_head(&mut Cursor::new(text)).unwrap().unwrap();
        assert_eq!(
            (h.method.as_str(), h.path.as_str(), h.len, h.auth.as_str(), h.close),
            ("POST", "/mcp", 12, "Bearer abc", true)
        );
    }

    #[test]
    fn failing_entropy_is_an_error_not_zeros() {
        assert!(make_token(no_entropy).is_err());
    }

    #[test]
    fn token_is_48_hex_digits() {
        let t = make_token(fixed_entropy).unwrap();
        assert_eq!(t.len(), 48);
        assert!(t.bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn content_length_must_be_a_number() {
        let text = "POST /mcp HTTP/1.1\r\nContent-Length: abc\r\n\r\n";
        let err = parse_head(&mut Cursor::new(text)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn second_content_length_is_refused() {
        let text = "POST /mcp HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 5\r\n\r\n";
        let err = parse_head(&mut Cursor::new(text)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_past_the_request_deadline_fails_without_blocking() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let _peer = l.accept().unwrap();
        let mut t = Timed { s: c, deadline: Some(Instant::now()) };
        let mut b = [0u8; 4];
        assert_eq!(t.read(&mut b).unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn open_http_slots_are_capped_and_released() {
        let mut held: Vec<Slot> = (0..MAX_HTTP_SESSIONS).map(|_| Slot::acquire().unwrap()).collect();
        assert!(Slot::acquire().is_none());
        held.pop();
        assert!(Slot::acquire().is_some());
    }

    #[test]
    fn bearer_must_match_the_token_exactly() {
        assert!(bearer_ok("Bearer secret-token", "secret-token"));
        assert!(!bearer_ok("Bearer secret-tokeN", "secret-token"));
        assert!(!bearer_ok("Bearer secret", "secret-token"));
        assert!(!bearer_ok("secret-token", "secret-token"));
        assert!(!bearer_ok("", "secret-token"));
    }
}
