//! `bifrost-mcp --serve`: the system service (bifrost-mcp.service, on by default). Agents connect over
//!   - MCP Streamable HTTP at http://<machine>:7420/mcp (POST JSON-RPC, JSON replies, keep-alive), with
//!     `Authorization: Bearer <token>` from /var/lib/bifrost-mcp/token (made on first start; required on every TCP request, local agents use the unix socket);
//!   - a Unix socket, /run/bifrost-mcp.sock (newline-delimited JSON-RPC, like stdio), for agents on the machine:
//!     root and the desktop account's group.
//! All sessions share one context behind a lock, so input from several agents is applied in order.

use crate::tools::Ctx;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TOKEN_PATH: &str = "/var/lib/bifrost-mcp/token";
pub const SOCKET: &str = "/run/bifrost-mcp.sock";
const MAX_BODY: usize = 8 << 20;

type Shared = Arc<Mutex<Ctx>>;

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
        let (ctx, token) = (ctx.clone(), token.clone());
        std::thread::spawn(move || http_session(s, ctx, &token));
    }
}

/// The bearer token, created (random, 0600) on first start.
fn token() -> String {
    if let Ok(t) = std::fs::read_to_string(TOKEN_PATH) {
        if t.trim().len() >= 32 {
            return t.trim().to_string();
        }
    }
    let mut b = [0u8; 24];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = std::fs::create_dir_all("/var/lib/bifrost-mcp");
    let _ = std::fs::write(TOKEN_PATH, format!("{t}\n"));
    let _ = std::fs::set_permissions(TOKEN_PATH, std::fs::Permissions::from_mode(0o600));
    t
}

fn unix_session(s: UnixStream, ctx: Shared) {
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    for line in BufReader::new(s).lines() {
        let Ok(line) = line else { break };
        let r = crate::handle(&mut ctx.lock().unwrap_or_else(|p| p.into_inner()), &line);
        if let Some(r) = r {
            if writeln!(w, "{r}").is_err() {
                break;
            }
        }
    }
}

fn http_session(s: TcpStream, ctx: Shared, token: &str) {
    let _ = s.set_nodelay(true);
    let _ = s.set_read_timeout(Some(Duration::from_secs(300)));
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut r = BufReader::new(s);
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let (mut len, mut auth, mut close) = (0usize, String::new(), false);
        loop {
            let mut h = String::new();
            if r.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').unwrap_or((h, ""));
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap_or(0),
                "authorization" => auth = v.trim().to_string(),
                "connection" => close = v.trim().eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
        if len > MAX_BODY {
            let _ = respond(&mut w, 413, "text/plain", b"too large", true);
            return;
        }
        let mut body = vec![0u8; len];
        if r.read_exact(&mut body).is_err() {
            return;
        }
        let ok = if !path.starts_with("/mcp") {
            respond(&mut w, 404, "text/plain", b"MCP endpoint is /mcp", close)
        } else if auth.strip_prefix("Bearer ").map(str::trim) != Some(token) {
            respond(&mut w, 401, "application/json", br#"{"error":"bearer token required (/var/lib/bifrost-mcp/token)"}"#, close)
        } else if method == "POST" {
            let text = String::from_utf8_lossy(&body);
            let reply = {
                let mut c = ctx.lock().unwrap_or_else(|p| p.into_inner());
                // a JSON-RPC batch is an array of messages
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Array(v)) => {
                        let out: Vec<_> = v.iter().filter_map(|m| crate::handle(&mut c, &m.to_string())).collect();
                        (!out.is_empty()).then(|| serde_json::Value::Array(out))
                    }
                    _ => crate::handle(&mut c, &text),
                }
            };
            match reply {
                Some(v) => respond(&mut w, 200, "application/json", v.to_string().as_bytes(), close),
                None => respond(&mut w, 202, "text/plain", b"", close),
            }
        } else if method == "DELETE" {
            respond(&mut w, 200, "text/plain", b"", close)
        } else {
            // no server-initiated stream: GET is not offered
            respond(&mut w, 405, "text/plain", b"POST JSON-RPC to /mcp", close)
        };
        if !ok || close {
            return;
        }
    }
}

fn respond(w: &mut TcpStream, code: u16, ctype: &str, body: &[u8], close: bool) -> bool {
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Payload Too Large",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nMcp-Session-Id: bifrost\r\nConnection: {}\r\n\r\n",
        body.len(),
        if close { "close" } else { "keep-alive" }
    );
    w.write_all(head.as_bytes()).and_then(|_| w.write_all(body)).and_then(|_| w.flush()).is_ok()
}
