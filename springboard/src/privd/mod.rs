//! `bifrost-privd`: the small root daemon behind the user session (docs/login-session.md).
//!
//! The shell runs as the installed user. Everything that needs root (Software Update, store installs, power,
//! time zone, the crash-loop latch) is a method on a Unix socket, `SOCK`, with fixed argv and validated
//! parameters. Wire format: a 4-byte big-endian length, then UTF-8 JSON, at most `MAX_FRAME` bytes.
//! Request `{"id":N,"method":"..","params":{..}}`; reply `{"id":N,"ok":true,"result":{..}}` or
//! `{"id":N,"ok":false,"error":{"code":"..","message":".."}}`.
//!
//! Login itself (PAM, lockout, the session token) lives behind the [`Gate`] trait, supplied by the login
//! track; `bin/privd.rs` installs its gate. Without one every authenticated method answers `locked`.

use bifrost_core::json::{self, Json};
use std::io::{self, Read, Write};

pub mod accounts;
pub mod auth;
pub mod disk;
pub mod gate;
pub mod loopdev;
pub mod methods;
pub mod pam;
pub mod proto;
pub mod server;

pub const SOCK: &str = "/run/bifrost/privd.sock";
pub const ROOT_SOCK: &str = "/run/bifrost/privd-root.sock";
pub const MAX_FRAME: usize = 64 * 1024;
pub const PROTO: u32 = 1;
/// Root-owned state (account file, apps/, boot_attempts).
pub const STATE_DIR: &str = "/var/lib/springboard";

/// The elevation scope of software installs: `package.install_file` and `package.install_repo` each spend one grant of
/// this scope (see `auth.elevate`).
pub const SCOPE_PACKAGE_INSTALL: &str = "package.install";

/// The elevation scope a method needs a fresh grant of, if any. A grant comes from `auth.elevate`, which re-checks the
/// administrator's password, and is spent by exactly one call.
pub fn elevation_scope(method: &str) -> Option<&'static str> {
    matches!(method, "package.install_file" | "package.install_repo").then_some(SCOPE_PACKAGE_INSTALL)
}

/// A failed call: one of unauthorized, locked, bad_params, wrong_password, locked_out, no_password, elevation_required, internal.
#[derive(Clone, Debug, PartialEq)]
pub struct PErr {
    pub code: &'static str,
    pub message: String,
    /// extra fields merged into the error object (`fails`, `retry_secs`)
    pub extra: Vec<(String, Json)>,
}

impl PErr {
    pub fn new(code: &'static str, message: impl Into<String>) -> PErr {
        PErr { code, message: message.into(), extra: vec![] }
    }
    pub fn bad(message: impl Into<String>) -> PErr {
        PErr::new("bad_params", message)
    }
    pub fn internal(message: impl Into<String>) -> PErr {
        PErr::new("internal", message)
    }
}

/// Who is on the other end of a request, as far as the server can tell from SO_PEERCRED and /proc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    /// connected to the root-only socket
    pub root_socket: bool,
    /// the peer is the console shell (`/usr/bin/springboard`) running as the installed account
    pub console_shell: bool,
}

/// The login state machine. `require_unlocked` is the check every AUTH method goes through.
pub trait Gate: Send + Sync {
    /// `login.account` result: username, full_name, uid, has_password, locked_out_secs, password_hint.
    fn account(&self) -> Result<Json, PErr>;
    /// `login.authenticate`: marks the session Unlocked and returns the token.
    fn authenticate(&self, password: &str) -> Result<String, PErr>;
    fn require_unlocked(&self, token: &str) -> Result<(), PErr>;
    /// `session.lock` / `session.logout`.
    fn lock(&self);
    /// `account.set_password`. `token` is the request's `t`, if any. A first password (empty hash) needs no
    /// token but only from the console shell; a set password needs the token and the old password; a locked
    /// hash is never changed from here.
    fn set_password(&self, token: Option<&str>, peer: Peer, old: &str, new: &str) -> Result<(), PErr>;
    /// `auth.elevate`: re-checks the administrator's password for a session that is already Unlocked (`token`) and
    /// returns a single-use grant for `scope`. The session token does not change.
    fn elevate(&self, token: &str, scope: &str, password: &str) -> Result<String, PErr>;
    /// Spends the grant a call of `scope` presented (`None` when it presented none). `elevation_required` when it is
    /// missing, spent, expired or for another scope.
    fn consume_grant(&self, token: &str, scope: &str, grant: Option<&str>) -> Result<(), PErr>;
}

/// The gate used when no login backend is installed: nothing authenticates.
pub struct ClosedGate;

impl Gate for ClosedGate {
    fn account(&self) -> Result<Json, PErr> {
        Err(PErr::new("no_password", "no login backend"))
    }
    fn authenticate(&self, _: &str) -> Result<String, PErr> {
        Err(PErr::new("no_password", "no login backend"))
    }
    fn require_unlocked(&self, _: &str) -> Result<(), PErr> {
        Err(PErr::new("locked", "session is locked"))
    }
    fn lock(&self) {}
    fn set_password(&self, _: Option<&str>, _: Peer, _: &str, _: &str) -> Result<(), PErr> {
        Err(PErr::internal("no login backend"))
    }
    fn elevate(&self, _: &str, _: &str, _: &str) -> Result<String, PErr> {
        Err(PErr::new("locked", "session is locked"))
    }
    fn consume_grant(&self, _: &str, _: &str, _: Option<&str>) -> Result<(), PErr> {
        Err(PErr::new("locked", "session is locked"))
    }
}

/// The login backend privd runs with: PAM (service `bifrost-login`) behind [`gate::AuthGate`]. Fails closed.
pub fn default_gate() -> std::sync::Arc<dyn Gate> {
    std::sync::Arc::new(gate::AuthGate::system())
}

pub fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn s(v: impl Into<String>) -> Json {
    Json::Str(v.into())
}

/// Serialises `v` as compact JSON.
pub fn to_string(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => format!("{}", *n as i64),
        Json::Num(n) if n.is_finite() => n.to_string(),
        Json::Num(_) => "null".into(),
        Json::Str(t) => json::quote(t),
        Json::Arr(a) => format!("[{}]", a.iter().map(to_string).collect::<Vec<_>>().join(",")),
        Json::Obj(o) => format!("{{{}}}", o.iter().map(|(k, v)| format!("{}:{}", json::quote(k), to_string(v))).collect::<Vec<_>>().join(",")),
    }
}

pub fn reply_ok(id: u64, result: &Json) -> String {
    format!("{{\"id\":{id},\"ok\":true,\"result\":{}}}", to_string(result))
}

pub fn reply_err(id: u64, e: &PErr) -> String {
    let mut f = vec![("code".to_string(), s(e.code)), ("message".to_string(), s(e.message.clone()))];
    f.extend(e.extra.iter().cloned());
    format!("{{\"id\":{id},\"ok\":false,\"error\":{}}}", to_string(&Json::Obj(f)))
}

/// Writes one frame.
pub fn write_frame(w: &mut impl Write, body: &str) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(body.as_bytes());
    w.write_all(&buf)?;
    w.flush()
}

/// Reads one frame; `Ok(None)` at a clean end of stream. A length over `MAX_FRAME` is `InvalidData`
/// and the payload is not read.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<String>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    String::from_utf8(body).map(Some).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame is not UTF-8"))
}

/// The installed account's username from `STATE_DIR/account` (`username=`).
pub fn account_name() -> Option<String> {
    let text = std::fs::read_to_string(format!("{STATE_DIR}/account")).ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("username="))?.trim().to_string();
    let ok = !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    ok.then_some(name)
}

/// (uid, gid, home) of `name` from passwd text; uid 0 is refused.
pub fn parse_passwd(passwd: &str, name: &str) -> Option<(u32, u32, String)> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() < 7 || f[0] != name {
            return None;
        }
        let (uid, gid) = (f[2].parse::<u32>().ok()?, f[3].parse::<u32>().ok()?);
        (uid != 0 && f[5].starts_with("/home/")).then(|| (uid, gid, f[5].to_string()))
    })
}

/// The installed account's (uid, gid, home).
pub fn account_ids() -> Option<(u32, u32, String)> {
    parse_passwd(&std::fs::read_to_string("/etc/passwd").ok()?, &account_name()?)
}

/// The gid of group `name` from group text.
pub fn parse_group(group: &str, name: &str) -> Option<u32> {
    group.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 3 && f[0] == name).then(|| f[2].parse().ok()).flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_cap() {
        let mut buf = Vec::new();
        write_frame(&mut buf, "{\"a\":1}").unwrap();
        assert_eq!(&buf[..4], &[0, 0, 0, 7]);
        assert_eq!(read_frame(&mut &buf[..]).unwrap().as_deref(), Some("{\"a\":1}"));
        assert_eq!(read_frame(&mut &b""[..]).unwrap(), None);
        let big = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert_eq!(read_frame(&mut &big[..]).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(write_frame(&mut Vec::new(), &"x".repeat(MAX_FRAME + 1)).is_err());
    }

    #[test]
    fn replies_serialise() {
        assert_eq!(reply_ok(3, &obj(vec![("v", Json::Num(1.0))])), "{\"id\":3,\"ok\":true,\"result\":{\"v\":1}}");
        let mut e = PErr::new("wrong_password", "no");
        e.extra.push(("retry_secs".into(), Json::Num(60.0)));
        let r = json::parse(&reply_err(4, &e)).unwrap();
        assert_eq!(r.get("error").and_then(|e| e.get("retry_secs")).and_then(Json::as_u64), Some(60));
    }

    #[test]
    fn passwd_refuses_root_and_foreign_homes() {
        let p = "root:x:0:0::/root:/bin/bash\nolli:x:1000:1000:O:/home/olli:/bin/bash\nsvc:x:5:5::/var/svc:/bin/false\n";
        assert_eq!(parse_passwd(p, "olli"), Some((1000, 1000, "/home/olli".into())));
        assert_eq!(parse_passwd(p, "root"), None);
        assert_eq!(parse_passwd(p, "svc"), None);
        assert_eq!(parse_group("bifrost-session:x:970:olli\n", "bifrost-session"), Some(970));
    }
}
