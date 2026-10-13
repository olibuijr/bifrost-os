//! The socket server: peer checks, framing, one thread per connection, and the UNAUTH / AUTH dispatch table.

use super::{obj, read_frame, reply_err, reply_ok, s, write_frame, Gate, PErr, Peer, PROTO};
use bifrost_core::json::{self, Json};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Concurrent connections served at once; more are dropped (an admitted peer cannot exhaust threads).
pub const MAX_CONNS: usize = 16;
/// Idle time before a connection is closed. Clients open one connection per call.
const READ_TIMEOUT_SECS: u64 = 30;
static CONNS: AtomicUsize = AtomicUsize::new(0);

/// The binary that is the console shell.
const SHELL_EXE: &str = "/usr/bin/springboard";

#[repr(C)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

extern "C" {
    fn getsockopt(fd: i32, level: i32, name: i32, val: *mut Ucred, len: *mut u32) -> i32;
    fn geteuid() -> u32;
    fn umask(mask: u32) -> u32;
    fn chown(path: *const std::os::raw::c_char, uid: u32, gid: u32) -> i32;
}

const SOL_SOCKET: i32 = 1;
const SO_PEERCRED: i32 = 17;

/// Executes the root methods; the real one is `methods::RootExec`, tests use a fake.
pub trait Exec: Send + Sync {
    fn call(&self, method: &str, params: &Json) -> Result<Json, PErr>;
}

pub struct Config {
    pub sock: PathBuf,
    /// the root-only socket (0600) for ssh tools and MCP
    pub root_sock: Option<PathBuf>,
    /// group that may open `sock` (`bifrost-session`)
    pub group_gid: Option<u32>,
    /// the installed account's uid; `None` admits root only
    pub allowed_uid: Option<u32>,
    /// require /proc/<pid>/exe to be one of `exes` for non-root peers
    pub check_exe: bool,
    pub exes: Vec<String>,
}

impl Config {
    pub fn system(allowed_uid: Option<u32>, group_gid: Option<u32>) -> Config {
        Config {
            sock: super::SOCK.into(),
            root_sock: Some(super::ROOT_SOCK.into()),
            group_gid,
            allowed_uid,
            check_exe: true,
            exes: vec!["/usr/bin/springboard".into(), "/usr/bin/bifrost-mcp".into()],
        }
    }
}

/// (pid, uid) of the process on the other end.
pub fn peer(stream: &UnixStream) -> io::Result<(i32, u32)> {
    let mut c = Ucred { pid: 0, uid: u32::MAX, gid: 0 };
    let mut len = std::mem::size_of::<Ucred>() as u32;
    // SAFETY: `c` and `len` are valid for the call; the fd is an open socket.
    let r = unsafe { getsockopt(stream.as_raw_fd(), SOL_SOCKET, SO_PEERCRED, &mut c, &mut len) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((c.pid, c.uid))
}

/// `/proc/<pid>/exe` without the " (deleted)" suffix a package update leaves on a running binary.
fn exe_of(pid: i32) -> Option<String> {
    let p = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?.to_string_lossy().into_owned();
    Some(p.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(p))
}

/// Admits or refuses a peer; on admission says whether it is the console shell.
fn admit(cfg: &Config, stream: &UnixStream, root_only: bool) -> Option<Peer> {
    let (pid, uid) = peer(stream).ok()?;
    if uid == 0 {
        return Some(Peer { root_socket: root_only, console_shell: false });
    }
    if root_only || cfg.allowed_uid != Some(uid) {
        eprintln!("privd: refused uid {uid} (pid {pid})");
        return None;
    }
    let exe = exe_of(pid);
    if cfg.check_exe && !exe.as_ref().is_some_and(|e| cfg.exes.contains(e)) {
        eprintln!("privd: refused uid {uid} pid {pid}: exe {exe:?} is not allowed");
        return None;
    }
    // without the exe check (host tests) every admitted peer stands in for the shell
    Some(Peer { root_socket: false, console_shell: !cfg.check_exe || exe.as_deref() == Some(SHELL_EXE) })
}

/// Methods allowed on the lock screen. `system.boot_attempt` is here because the crash-loop latch must
/// count a start before anyone can log in; it only writes one fixed counter file.
fn is_unauth(method: &str) -> bool {
    matches!(
        method,
        "system.ping" | "login.account" | "login.authenticate" | "power.shutdown" | "power.restart" | "power.sleep" | "session.lock" | "session.logout" | "system.boot_attempt"
    )
}

/// Routes one request. `peer.root_socket` is the root socket: its peers are root, so no token is needed.
pub fn dispatch(gate: &dyn Gate, exec: &dyn Exec, peer: Peer, method: &str, params: &Json) -> Result<Json, PErr> {
    match method {
        "system.ping" => return Ok(obj(vec![("version", s(env!("CARGO_PKG_VERSION"))), ("proto", Json::Num(PROTO as f64))])),
        "login.account" => return gate.account(),
        "login.authenticate" => {
            let pw = params.get("password").and_then(Json::as_str).ok_or_else(|| PErr::bad("password"))?;
            return gate.authenticate(pw).map(|t| obj(vec![("ok", Json::Bool(true)), ("token", s(t))]));
        }
        "session.lock" | "session.logout" => {
            gate.lock();
            return Ok(obj(vec![]));
        }
        _ => {}
    }
    let token = params.get("t").and_then(Json::as_str);
    if method == "account.set_password" {
        // before the token check: the first password of a fresh account needs no token (the gate decides
        // from the hash and the peer, and only an empty hash from the console shell qualifies)
        let new = params.get("new").and_then(Json::as_str).ok_or_else(|| PErr::bad("new"))?;
        let old = params.get("old").and_then(Json::as_str).unwrap_or("");
        return gate.set_password(token, peer, old, new).map(|_| obj(vec![]));
    }
    // `begin` counts a start before anyone can log in; `stable` clears the crash-loop latch, so it needs the
    // session token or a crash-looping shell could reset its own latch
    let stable = method == "system.boot_attempt" && params.get("event").and_then(Json::as_str) == Some("stable");
    if (!is_unauth(method) || stable) && !peer.root_socket {
        gate.require_unlocked(token.ok_or_else(|| PErr::new("unauthorized", "missing token"))?)?;
    }
    exec.call(method, params)
}

/// Handles one request frame and returns the reply frame.
pub fn handle_request(gate: &dyn Gate, exec: &dyn Exec, peer: Peer, text: &str) -> String {
    let req = match json::parse(text) {
        Ok(j @ Json::Obj(_)) => j,
        _ => return reply_err(0, &PErr::bad("request is not a JSON object")),
    };
    let id = req.get("id").and_then(Json::as_u64).unwrap_or(0);
    let Some(method) = req.get("method").and_then(Json::as_str) else { return reply_err(id, &PErr::bad("missing method")) };
    let empty = Json::Obj(vec![]);
    let params = req.get("params").unwrap_or(&empty);
    let r = dispatch(gate, exec, peer, method, params);
    // the journal gets the method and the outcome, never the parameters (passwords travel here)
    // the method is the client's string: escaped and capped, so it cannot forge journal lines
    let shown: String = method.chars().take(64).collect();
    match &r {
        Ok(_) => eprintln!("privd: {shown:?} ok"),
        Err(e) => eprintln!("privd: {shown:?} -> {}", e.code),
    }
    match r {
        Ok(v) => reply_ok(id, &v),
        Err(e) => reply_err(id, &e),
    }
}

fn serve_conn(mut stream: UnixStream, gate: Arc<dyn Gate>, exec: Arc<dyn Exec>, peer: Peer) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(READ_TIMEOUT_SECS)));
    loop {
        let reply = match read_frame(&mut stream) {
            Ok(Some(text)) => handle_request(gate.as_ref(), exec.as_ref(), peer, &text),
            Ok(None) => return,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let _ = write_frame(&mut stream, &reply_err(0, &PErr::bad(e.to_string())));
                return;
            }
            Err(_) => return,
        };
        if write_frame(&mut stream, &reply).is_err() {
            return;
        }
    }
}

/// Binds `path` as a socket owned by root with `mode` (and `gid` as group when given).
fn bind(path: &Path, mode: u32, gid: Option<u32>) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    // SAFETY: umask only changes this process's creation mask.
    let old = unsafe { umask(0o177) };
    let l = UnixListener::bind(path);
    unsafe { umask(old) };
    let l = l?;
    if let Some(g) = gid {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
        // SAFETY: `c` is a valid NUL-terminated path.
        if unsafe { chown(c.as_ptr(), 0, g) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(l)
}

/// The listener systemd passed (`LISTEN_FDS=1`, fd 3), when this process was socket-activated.
fn activated() -> Option<UnixListener> {
    let ok = std::env::var("LISTEN_PID").ok()?.parse::<u32>().ok()? == std::process::id() && std::env::var("LISTEN_FDS").ok()?.trim() == "1";
    // SAFETY: systemd guarantees fd 3 is an open listening socket when LISTEN_PID matches.
    ok.then(|| unsafe { UnixListener::from_raw_fd(3) })
}

fn accept_loop(l: UnixListener, cfg: Arc<Config>, gate: Arc<dyn Gate>, exec: Arc<dyn Exec>, root_only: bool) {
    for stream in l.incoming() {
        let Ok(stream) = stream else { continue };
        let Some(peer) = admit(&cfg, &stream, root_only) else { continue };
        if CONNS.fetch_add(1, Ordering::SeqCst) >= MAX_CONNS {
            CONNS.fetch_sub(1, Ordering::SeqCst);
            eprintln!("privd: too many connections; dropped one");
            continue;
        }
        let (gate, exec) = (gate.clone(), exec.clone());
        let spawned = std::thread::Builder::new().name("privd-conn".into()).spawn(move || {
            serve_conn(stream, gate, exec, peer);
            CONNS.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            CONNS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Starts the accept threads and returns at once (the daemon's main then parks).
pub fn serve(cfg: Config, gate: Arc<dyn Gate>, exec: Arc<dyn Exec>) -> io::Result<Vec<std::thread::JoinHandle<()>>> {
    let main = match activated() {
        Some(l) => l,
        None => bind(&cfg.sock, 0o660, cfg.group_gid)?,
    };
    let root = cfg.root_sock.as_ref().map(|p| bind(p, 0o600, None)).transpose()?;
    let cfg = Arc::new(cfg);
    let mut hs = vec![];
    {
        let (c, g, e) = (cfg.clone(), gate.clone(), exec.clone());
        hs.push(std::thread::Builder::new().name("privd-accept".into()).spawn(move || accept_loop(main, c, g, e, false))?);
    }
    if let Some(l) = root {
        hs.push(std::thread::Builder::new().name("privd-accept-root".into()).spawn(move || accept_loop(l, cfg, gate, exec, true))?);
    }
    Ok(hs)
}

/// The current effective uid (tests admit themselves with it).
pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    struct TestGate {
        unlocked: Mutex<bool>,
    }
    impl Gate for TestGate {
        fn account(&self) -> Result<Json, PErr> {
            Ok(obj(vec![("username", s("olli"))]))
        }
        fn authenticate(&self, pw: &str) -> Result<String, PErr> {
            if pw == "good" {
                *self.unlocked.lock().unwrap() = true;
                Ok("tok".into())
            } else {
                Err(PErr::new("wrong_password", "no"))
            }
        }
        fn require_unlocked(&self, t: &str) -> Result<(), PErr> {
            if *self.unlocked.lock().unwrap() && t == "tok" {
                Ok(())
            } else {
                Err(PErr::new("locked", "locked"))
            }
        }
        fn lock(&self) {
            *self.unlocked.lock().unwrap() = false;
        }
        fn set_password(&self, _: Option<&str>, _: Peer, _: &str, _: &str) -> Result<(), PErr> {
            Ok(())
        }
    }

    struct FakeExec;
    impl Exec for FakeExec {
        fn call(&self, method: &str, params: &Json) -> Result<Json, PErr> {
            match method {
                "time.set_ntp" => params.get("on").map(|_| obj(vec![])).ok_or_else(|| PErr::bad("on")),
                _ => Err(PErr::bad(format!("unknown method {method}"))),
            }
        }
    }

    fn start(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("privd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("p.sock");
        let cfg = Config { sock: sock.clone(), root_sock: None, group_gid: None, allowed_uid: Some(euid()), check_exe: false, exes: vec![] };
        serve(cfg, Arc::new(TestGate { unlocked: Mutex::new(false) }), Arc::new(FakeExec)).unwrap();
        sock
    }

    fn call(c: &mut UnixStream, body: &str) -> Json {
        write_frame(c, body).unwrap();
        json::parse(&read_frame(c).unwrap().unwrap()).unwrap()
    }

    fn code(r: &Json) -> String {
        r.get("error").and_then(|e| e.str_field("code")).unwrap_or_default()
    }

    #[test]
    fn framing_auth_and_params() {
        let mut c = UnixStream::connect(start("flow")).unwrap();
        let r = call(&mut c, r#"{"id":1,"method":"system.ping"}"#);
        assert_eq!(r.get("ok"), Some(&Json::Bool(true)));
        assert_eq!(r.get("id").and_then(Json::as_u64), Some(1));
        // AUTH method before login: no token, then a token that the gate does not know
        assert_eq!(code(&call(&mut c, r#"{"id":2,"method":"time.set_ntp","params":{"on":true}}"#)), "unauthorized");
        assert_eq!(code(&call(&mut c, r#"{"id":3,"method":"time.set_ntp","params":{"on":true,"t":"x"}}"#)), "locked");
        assert_eq!(code(&call(&mut c, r#"{"id":4,"method":"login.authenticate","params":{"password":"bad"}}"#)), "wrong_password");
        let r = call(&mut c, r#"{"id":5,"method":"login.authenticate","params":{"password":"good"}}"#);
        assert_eq!(r.get("result").and_then(|r| r.str_field("token")).as_deref(), Some("tok"));
        // bad params and unknown methods once unlocked
        assert_eq!(code(&call(&mut c, r#"{"id":6,"method":"time.set_ntp","params":{"t":"tok"}}"#)), "bad_params");
        assert_eq!(code(&call(&mut c, r#"{"id":7,"method":"nope","params":{"t":"tok"}}"#)), "bad_params");
        assert_eq!(call(&mut c, r#"{"id":8,"method":"time.set_ntp","params":{"on":false,"t":"tok"}}"#).get("ok"), Some(&Json::Bool(true)));
        // session.lock revokes the token
        call(&mut c, r#"{"id":9,"method":"session.lock"}"#);
        assert_eq!(code(&call(&mut c, r#"{"id":10,"method":"time.set_ntp","params":{"on":true,"t":"tok"}}"#)), "locked");
    }

    #[test]
    fn garbage_is_bad_params() {
        let mut c = UnixStream::connect(start("garbage")).unwrap();
        assert_eq!(code(&call(&mut c, "not json")), "bad_params");
        assert_eq!(code(&call(&mut c, r#"{"id":2}"#)), "bad_params");
    }

    #[test]
    fn oversize_frame_is_refused_and_closes() {
        let mut c = UnixStream::connect(start("big")).unwrap();
        c.write_all(&(super::super::MAX_FRAME as u32 + 1).to_be_bytes()).unwrap();
        let r = json::parse(&read_frame(&mut c).unwrap().unwrap()).unwrap();
        assert_eq!(code(&r), "bad_params");
        assert_eq!(read_frame(&mut c).unwrap(), None);
    }

    #[test]
    fn root_socket_needs_no_token() {
        let r = handle_request(&TestGate { unlocked: Mutex::new(false) }, &FakeExec, Peer { root_socket: true, console_shell: false }, r#"{"id":1,"method":"time.set_ntp","params":{"on":true}}"#);
        assert!(r.contains("\"ok\":true"));
    }

    #[test]
    fn stable_latch_reset_needs_the_token_but_begin_does_not() {
        let g = TestGate { unlocked: Mutex::new(false) };
        let peer = Peer::default();
        let stable = r#"{"id":1,"method":"system.boot_attempt","params":{"event":"stable"}}"#;
        assert!(handle_request(&g, &FakeExec, peer, stable).contains("unauthorized"));
        // begin gets past the token check (the fake exec then rejects the method)
        let begin = r#"{"id":2,"method":"system.boot_attempt","params":{"event":"begin"}}"#;
        assert!(handle_request(&g, &FakeExec, peer, begin).contains("bad_params"));
    }

    #[test]
    fn set_password_reaches_the_gate_without_a_token() {
        struct Rec(Mutex<Vec<(bool, bool)>>);
        impl Gate for Rec {
            fn account(&self) -> Result<Json, PErr> {
                Ok(Json::Null)
            }
            fn authenticate(&self, _: &str) -> Result<String, PErr> {
                Err(PErr::new("locked", "x"))
            }
            fn require_unlocked(&self, _: &str) -> Result<(), PErr> {
                Err(PErr::new("locked", "x"))
            }
            fn lock(&self) {}
            fn set_password(&self, t: Option<&str>, p: Peer, _: &str, _: &str) -> Result<(), PErr> {
                self.0.lock().unwrap().push((t.is_some(), p.console_shell));
                Err(PErr::new("unauthorized", "x"))
            }
        }
        let g = Rec(Mutex::new(vec![]));
        let body = r#"{"id":1,"method":"account.set_password","params":{"new":"longenough"}}"#;
        handle_request(&g, &FakeExec, Peer { root_socket: false, console_shell: true }, body);
        handle_request(&g, &FakeExec, Peer::default(), body);
        assert_eq!(*g.0.lock().unwrap(), vec![(false, true), (false, false)]);
    }
}
