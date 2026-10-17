//! The socket server: peer checks, framing, one thread per connection, and the UNAUTH / AUTH dispatch table.

use super::{obj, read_frame, reply_err, reply_ok, s, write_frame, Gate, PErr, Peer, PROTO};
use bifrost_core::json::{self, Json};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
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
    if method == "auth.elevate" {
        // a fresh password check by a session that is already unlocked; the reply is a single-use grant. The root
        // socket's peers are root and need none.
        if peer.root_socket {
            return Err(PErr::bad("the root socket needs no elevation"));
        }
        let token = token.ok_or_else(|| PErr::new("unauthorized", "missing token"))?;
        let scope = params.get("scope").and_then(Json::as_str).ok_or_else(|| PErr::bad("scope"))?;
        let pw = params.get("password").and_then(Json::as_str).ok_or_else(|| PErr::bad("password"))?;
        return gate.elevate(token, scope, pw).map(|g| obj(vec![("grant", s(g))]));
    }
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
    // installing software needs the administrator's password again, not only the session token: the call spends the
    // grant `auth.elevate` issued, so a shell that holds the token cannot install on its own
    if let (Some(scope), false) = (super::elevation_scope(method), peer.root_socket) {
        let token = token.ok_or_else(|| PErr::new("unauthorized", "missing token"))?;
        gate.consume_grant(token, scope, params.get("grant").and_then(Json::as_str))?;
    }
    exec.call(method, params)
}

/// The audit record of one handled request, one line for the journal: the method, the outcome and which door the
/// request came through (`root` socket, the console `shell`, any `other` admitted peer). Every privileged call leaves
/// one, allowed or refused, so the journal is the trail of what the root daemon was asked to do. It never carries the
/// parameters (passwords travel there). The method is the client's string: it is capped and quoted with Debug escapes,
/// so it cannot forge a second journal line.
pub fn audit_line(peer: Peer, method: &str, outcome: &Result<Json, PErr>) -> String {
    let shown: String = method.chars().take(64).collect();
    let via = if peer.root_socket {
        "root"
    } else if peer.console_shell {
        "shell"
    } else {
        "other"
    };
    match outcome {
        Ok(_) => format!("privd: {shown:?} ok via {via}"),
        Err(e) => format!("privd: {shown:?} -> {} via {via}", e.code),
    }
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
    eprintln!("{}", audit_line(peer, method, &r));
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
///
/// The socket is made inside a private staging directory (mode 0700) and renamed into place, so it is never
/// reachable with the default creation mode. This does not touch the process umask: `umask` is process-wide, and
/// changing it here raced with other threads creating directories (the tests run in parallel) and made their
/// directories unsearchable.
fn bind(path: &Path, mode: u32, gid: Option<u32>) -> io::Result<UnixListener> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let _ = std::fs::remove_file(path);
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let stage = parent.join(format!(".{name}.stage"));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::DirBuilder::new().mode(0o700).create(&stage)?;
    let staged = stage.join("s");
    let l = UnixListener::bind(&staged)?;
    if let Some(g) = gid {
        let c = std::ffi::CString::new(staged.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
        // SAFETY: `c` is a valid NUL-terminated path.
        if unsafe { chown(c.as_ptr(), 0, g) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&staged, path)?;
    let _ = std::fs::remove_dir(&stage);
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
        grant: Mutex<Option<String>>,
    }
    impl TestGate {
        fn new() -> TestGate {
            TestGate { unlocked: Mutex::new(false), grant: Mutex::new(None) }
        }
    }
    impl Gate for TestGate {
        fn elevate(&self, t: &str, scope: &str, pw: &str) -> Result<String, PErr> {
            self.require_unlocked(t)?;
            if scope != super::super::SCOPE_PACKAGE_INSTALL {
                return Err(PErr::bad("scope"));
            }
            if pw != "good" {
                return Err(PErr::new("wrong_password", "no"));
            }
            *self.grant.lock().unwrap() = Some("grant-1".into());
            Ok("grant-1".into())
        }
        fn consume_grant(&self, t: &str, _: &str, grant: Option<&str>) -> Result<(), PErr> {
            self.require_unlocked(t)?;
            let held = self.grant.lock().unwrap().take();
            if held.is_some() && held.as_deref() == grant {
                Ok(())
            } else {
                Err(PErr::new("elevation_required", "authenticate again"))
            }
        }
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
                "package.install_repo" | "package.install_file" => Ok(obj(vec![])),
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
        serve(cfg, Arc::new(TestGate::new()), Arc::new(FakeExec)).unwrap();
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
    fn every_call_leaves_one_audit_line_without_its_parameters() {
        let shell = Peer { root_socket: false, console_shell: true };
        assert_eq!(audit_line(shell, "power.sleep", &Ok(obj(vec![]))), r#"privd: "power.sleep" ok via shell"#);
        let refused = audit_line(Peer::default(), "account.set_password", &Err(PErr::new("wrong_password", "x")));
        assert_eq!(refused, r#"privd: "account.set_password" -> wrong_password via other"#);
        assert!(audit_line(Peer { root_socket: true, console_shell: false }, "update.check", &Ok(obj(vec![]))).ends_with("via root"));
        // a hostile method name stays on one line and is capped
        let forged = audit_line(shell, "x\nprivd: \"power.shutdown\" ok via root", &Ok(obj(vec![])));
        assert_eq!(forged.lines().count(), 1);
        let long = audit_line(shell, &"m".repeat(500), &Ok(obj(vec![])));
        assert!(long.len() < 100);
    }

    #[test]
    fn the_audit_line_of_a_handled_request_never_shows_the_password() {
        let g = TestGate::new();
        let req = json::parse(r#"{"id":1,"method":"login.authenticate","params":{"password":"hunter2-secret"}}"#).unwrap();
        let outcome = dispatch(&g, &FakeExec, Peer::default(), "login.authenticate", req.get("params").unwrap());
        assert_eq!(outcome.as_ref().unwrap_err().code, "wrong_password");
        let line = audit_line(Peer::default(), "login.authenticate", &outcome);
        assert_eq!(line, r#"privd: "login.authenticate" -> wrong_password via other"#);
        assert!(!line.contains("hunter2"));
    }

    #[test]
    fn root_socket_needs_no_token() {
        let r = handle_request(&TestGate::new(), &FakeExec, Peer { root_socket: true, console_shell: false }, r#"{"id":1,"method":"time.set_ntp","params":{"on":true}}"#);
        assert!(r.contains("\"ok\":true"));
    }

    #[test]
    fn stable_latch_reset_needs_the_token_but_begin_does_not() {
        let g = TestGate::new();
        let peer = Peer::default();
        let stable = r#"{"id":1,"method":"system.boot_attempt","params":{"event":"stable"}}"#;
        assert!(handle_request(&g, &FakeExec, peer, stable).contains("unauthorized"));
        // begin gets past the token check (the fake exec then rejects the method)
        let begin = r#"{"id":2,"method":"system.boot_attempt","params":{"event":"begin"}}"#;
        assert!(handle_request(&g, &FakeExec, peer, begin).contains("bad_params"));
    }

    #[test]
    fn installing_software_spends_a_fresh_elevation_grant() {
        let g = TestGate::new();
        let peer = Peer { root_socket: false, console_shell: true };
        let run = |body: &str| json::parse(&handle_request(&g, &FakeExec, peer, body)).unwrap();
        let install = r#"{"id":1,"method":"package.install_repo","params":{"names":["htop"],"t":"tok"}}"#;
        // locked: nothing, with or without a grant
        assert_eq!(code(&run(install)), "locked");
        run(r#"{"id":2,"method":"login.authenticate","params":{"password":"good"}}"#);
        // the session token alone is not enough: the shell cannot install without a fresh authentication
        assert_eq!(code(&run(install)), "elevation_required");
        let file = r#"{"id":3,"method":"package.install_file","params":{"path":"/x/a-1-1-any.pkg.tar.zst","t":"tok"}}"#;
        assert_eq!(code(&run(file)), "elevation_required");
        // a wrong password issues no grant
        let bad = r#"{"id":4,"method":"auth.elevate","params":{"password":"nope","scope":"package.install","t":"tok"}}"#;
        assert_eq!(code(&run(bad)), "wrong_password");
        assert_eq!(code(&run(install)), "elevation_required");
        // elevate needs the token
        assert_eq!(code(&run(r#"{"id":5,"method":"auth.elevate","params":{"password":"good","scope":"package.install"}}"#)), "unauthorized");
        // a right password issues one grant, and one install spends it
        let ok = run(r#"{"id":6,"method":"auth.elevate","params":{"password":"good","scope":"package.install","t":"tok"}}"#);
        assert_eq!(ok.get("result").and_then(|r| r.str_field("grant")).as_deref(), Some("grant-1"));
        let with = r#"{"id":7,"method":"package.install_repo","params":{"names":["htop"],"t":"tok","grant":"grant-1"}}"#;
        assert_eq!(run(with).get("ok"), Some(&Json::Bool(true)));
        assert_eq!(code(&run(with)), "elevation_required", "a grant is good for one install");
        // a different grant string does not pass either
        run(r#"{"id":8,"method":"auth.elevate","params":{"password":"good","scope":"package.install","t":"tok"}}"#);
        let forged = r#"{"id":9,"method":"package.install_repo","params":{"names":["htop"],"t":"tok","grant":"other"}}"#;
        assert_eq!(code(&run(forged)), "elevation_required");
    }

    #[test]
    fn the_root_socket_installs_without_a_grant() {
        let g = TestGate::new();
        let root = Peer { root_socket: true, console_shell: false };
        let r = handle_request(&g, &FakeExec, root, r#"{"id":1,"method":"package.install_repo","params":{"names":["htop"]}}"#);
        assert!(r.contains("\"ok\":true"));
        let r = handle_request(&g, &FakeExec, root, r#"{"id":2,"method":"auth.elevate","params":{"password":"good","scope":"package.install"}}"#);
        assert!(r.contains("bad_params"), "root has nothing to elevate");
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
            fn elevate(&self, _: &str, _: &str, _: &str) -> Result<String, PErr> {
                Err(PErr::new("locked", "x"))
            }
            fn consume_grant(&self, _: &str, _: &str, _: Option<&str>) -> Result<(), PErr> {
                Err(PErr::new("locked", "x"))
            }
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
