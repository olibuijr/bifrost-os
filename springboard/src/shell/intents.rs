//! App Intents in the shell (`AppShortcut` in Search). Apps declare intents in `manifest.toml` as `[[intent]]`
//! tables (SDK: `bifrost_app::intents`); the shell reads them without launching the app, lists the runnable ones
//! in Search's "Shortcuts" section, and runs one by launching the app and calling `intent.<id>` on its control
//! socket (`control.sock` in the app's data directory, `bifrost_core::app_control_socket`; JSON-RPC 2.0, one request per
//! connection).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One `[[intent]]` table of a manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestIntent {
    pub id: String,
    pub title: String,
    pub description: String,
    /// `AppShortcut`: offered in Search
    pub shortcut: bool,
    /// `name:type` with a trailing `?` when optional, for example `due:date?`
    pub params: Vec<String>,
}

impl ManifestIntent {
    /// Search can run it with no input: flagged as a shortcut, a safe id, and no required parameter.
    pub fn runnable(&self) -> bool {
        self.shortcut && !self.title.is_empty() && !self.id.is_empty() && self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && self.params.iter().all(|p| p.ends_with('?'))
    }
}

/// One runnable shortcut intent of an installed app: what Search and the Shortcuts action library list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppShortcut {
    pub app_id: String,
    pub app_name: String,
    pub intent: ManifestIntent,
}

/// The runnable shortcut intents of the given installed apps, in install order and then manifest order. Apps that
/// declare none contribute nothing. Reads only what the manifests say; no app is launched.
pub fn runnable_shortcuts(apps: &[super::apphost::Installed]) -> Vec<AppShortcut> {
    apps.iter()
        .flat_map(|app| {
            app.manifest.intents.iter().filter(|i| i.runnable()).map(move |i| AppShortcut {
                app_id: app.manifest.id.clone(),
                app_name: app.manifest.name.clone(),
                intent: i.clone(),
            })
        })
        .collect()
}

/// The JSON-RPC request line that runs intent `id` with no parameters.
pub fn request_line(id: &str) -> String {
    request_line_with(id, &[])
}

/// The JSON-RPC request line that runs intent `id` with string parameters, `(name, value)` pairs.
pub fn request_line_with(id: &str, params: &[(&str, &str)]) -> String {
    let body: Vec<String> = params.iter().map(|(name, value)| format!("{}:{}", json_string(name), json_string(value))).collect();
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.{id}\",\"params\":{{{}}}}}\n", body.join(","))
}

/// A JSON string literal for `s`.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The control socket of a running app: `control.sock` in its data directory (`<root>/appdata/<app id>`), where the
/// SDK runtime binds it. The sandbox binds the data directory at its own path, so the shell sees the same socket.
pub fn socket_path(app_id: &str) -> PathBuf {
    bifrost_core::app_control_socket::socket_for(&super::apphost::root(), app_id)
}

/// What an RPC reply says: `Ok(dialog)` for a result, `Err(message)` for an error object.
pub fn parse_reply(reply: &str) -> Result<Option<String>, String> {
    let v = super::store::json::parse(reply.trim()).map_err(|e| format!("malformed reply: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(e.str_field("message").unwrap_or_else(|| "error".into()));
    }
    Ok(v.get("result").and_then(|r| r.str_field("dialog")))
}

/// Calls the intent, retrying while the app starts (up to `wait`). Blocking: run it off the UI thread.
pub fn run(app_id: &str, intent_id: &str, wait: Duration) -> Result<Option<String>, String> {
    run_at(&socket_path(app_id), &request_line(intent_id), wait)
}

/// Runs intent `intent_id` of a running app with string parameters (see [`request_line_with`]).
pub fn run_with(app_id: &str, intent_id: &str, params: &[(&str, &str)], wait: Duration) -> Result<Option<String>, String> {
    run_at(&socket_path(app_id), &request_line_with(intent_id, params), wait)
}

/// Sends one JSON-RPC request line to a running app's control socket and returns the raw reply. Retries the connect
/// while the app starts (up to `wait`). Blocking: run it off the UI thread. Services use it for `service.<method>`.
pub fn exchange(app_id: &str, line: &str, wait: Duration) -> Result<String, String> {
    exchange_at(&socket_path(app_id), line, wait)
}

/// The installed apps that answer visual searches: those that declare `[[intent]] id = "visual-search"` in their
/// manifest (`bifrost_ui::visual_search_wire::PROVIDER_INTENT`). Reads only what the manifests say.
pub fn visual_search_apps(apps: &[super::apphost::Installed]) -> Vec<String> {
    apps.iter()
        .filter(|a| a.manifest.intents.iter().any(|i| i.id == bifrost_ui::visual_search_wire::PROVIDER_INTENT))
        .map(|a| a.manifest.id.clone())
        .collect()
}

/// Asks the app listening at `path` which of its content matches `scene` (`visual_search.search`, see
/// `bifrost_ui::visual_search_wire`). Every match is stamped with `app_id`, whatever the app wrote, so an app cannot
/// answer for another one. An app that cannot be reached within `wait`, refuses, or answers badly gives no matches.
/// Blocking: run it off the UI thread.
pub fn visual_search_at(path: &std::path::Path, app_id: &str, scene: &bifrost_ui::visual_search::Scene, wait: Duration) -> Vec<bifrost_ui::visual_search::Match> {
    let line = bifrost_ui::visual_search_wire::request_line(scene, 1);
    let Ok(reply) = exchange_at(path, &line, wait) else { return Vec::new() };
    let mut found = bifrost_ui::visual_search_wire::matches_from_reply(&reply).unwrap_or_default();
    for m in &mut found {
        m.app = app_id.to_string();
    }
    found
}

/// Delivers `scene` to every running app that answers visual searches and returns the merged matches, best first, at
/// most `MAX_MATCHES`. An app that is not running is not started and is skipped. Blocking (each answering app may take
/// a few seconds): run it off the UI thread.
pub fn visual_search(apps: &[super::apphost::Installed], scene: &bifrost_ui::visual_search::Scene) -> Vec<bifrost_ui::visual_search::Match> {
    let mut all = Vec::new();
    for id in visual_search_apps(apps) {
        all.extend(visual_search_at(&socket_path(&id), &id, scene, Duration::ZERO));
    }
    bifrost_ui::visual_search::merge(all, bifrost_ui::visual_search::MAX_MATCHES)
}

fn run_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<Option<String>, String> {
    parse_reply(&exchange_at(path, line, wait)?)
}

fn exchange_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<String, String> {
    let deadline = Instant::now() + wait;
    let account = super::screenshot::account_owner();
    let mut stream = loop {
        // the owner is looked up on every attempt: the socket appears (or is replaced) while the app starts
        let dial_as = dial_identity(socket_owner(path), socket_owner(path.parent().unwrap_or(path)), account);
        match dial(path, dial_as) {
            Ok(s) => break s,
            Err(e) if Instant::now() >= deadline => return Err(format!("{}: {e}", path.display())),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply)
}

/// The uid that owns `path` itself (a symbolic link is not followed), or None when it cannot be read.
fn socket_owner(path: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| m.uid())
}

/// Whose credentials dial a control socket. `account` is the installed account's `(uid, gid)` when the shell runs as
/// root ([`super::screenshot::account_owner`]), else None and the shell dials as itself. A root shell dials as root
/// only when the socket and the folder holding it are both root's: that is an app the shell started as root (the
/// launch route without the sandbox), whose socket is root-only. Every other socket, above all one the account owns
/// (a sandboxed app, whose user namespace maps only the account's uid, so root would arrive as the overflow uid and be
/// refused) or one that is missing or unreadable, is dialled with the account's credentials: the least privilege that
/// reaches an app, and an app cannot point a root dial elsewhere by swapping its socket for a link.
fn dial_identity(socket_owner: Option<u32>, dir_owner: Option<u32>, account: Option<(u32, u32)>) -> Option<(u32, u32)> {
    match (socket_owner, dir_owner) {
        (Some(0), Some(0)) => None,
        _ => account,
    }
}

/// Connects to an app's control socket, as `as_account` (`(uid, gid)`, see [`dial_identity`]) or, with None, as the
/// process's own user. The connect never waits: an app that does not accept (its backlog full) fails the attempt with
/// `WouldBlock` at once, and [`exchange_at`] retries until its deadline, so a dial is bounded and never holds the
/// credential lock for long.
fn dial(path: &std::path::Path, as_account: Option<(u32, u32)>) -> std::io::Result<UnixStream> {
    match as_account {
        #[cfg(target_os = "linux")]
        Some((uid, gid)) => account_dial::connect_as(path, uid, gid),
        _ => connect_now(path),
    }
}

/// A connect that does not wait for the app to accept. On Linux a non-blocking connect on a Unix stream socket either
/// completes at once or fails with `EAGAIN` when the listener's backlog is full (it never goes `EINPROGRESS`).
#[cfg(target_os = "linux")]
fn connect_now(path: &std::path::Path) -> std::io::Result<UnixStream> {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    const AF_UNIX: i32 = 1;
    const SOCK_STREAM: i32 = 1;
    const SOCK_NONBLOCK: i32 = 0o4000;
    const SOCK_CLOEXEC: i32 = 0o2000000;
    const EAGAIN: i32 = 11;
    const EINPROGRESS: i32 = 115;

    #[repr(C)]
    struct SockaddrUn {
        sun_family: u16,
        sun_path: [u8; 108],
    }
    extern "C" {
        fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
        fn connect(fd: i32, addr: *const SockaddrUn, len: u32) -> i32;
    }

    let bytes = path.as_os_str().as_bytes();
    let mut addr = SockaddrUn { sun_family: AF_UNIX as u16, sun_path: [0; 108] };
    if bytes.is_empty() || bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "control socket path does not fit a Unix socket address"));
    }
    addr.sun_path[..bytes.len()].copy_from_slice(bytes);
    // the family, the path and its terminating NUL (`sun_path` is zeroed past the path)
    let len = (std::mem::size_of::<u16>() + bytes.len() + 1) as u32;
    // SAFETY: plain integers; the returned descriptor is owned right below.
    let fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor nothing else owns; OwnedFd closes it on every path out.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: `addr` is a live sockaddr_un and `len` stays within it (checked above).
    if unsafe { connect(owned.as_raw_fd(), &addr, len) } != 0 {
        let e = io::Error::last_os_error();
        return Err(match e.raw_os_error() {
            Some(EAGAIN) | Some(EINPROGRESS) => io::Error::new(io::ErrorKind::WouldBlock, "the app is not accepting connections on its control socket"),
            _ => e,
        });
    }
    let stream = UnixStream::from(owned);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

#[cfg(not(target_os = "linux"))]
fn connect_now(path: &std::path::Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(path)
}

/// The account-credential connect: one thread whose own credentials (not the process's) become the account's.
#[cfg(target_os = "linux")]
mod account_dial {
    use std::io;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    #[cfg(target_arch = "x86_64")]
    mod nr {
        pub const SETGROUPS: i64 = 116;
        pub const SETRESUID: i64 = 117;
        pub const SETRESGID: i64 = 119;
    }
    #[cfg(target_arch = "aarch64")]
    mod nr {
        pub const SETGROUPS: i64 = 159;
        pub const SETRESUID: i64 = 147;
        pub const SETRESGID: i64 = 149;
    }

    extern "C" {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn syscall(n: i64, ...) -> i64;
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn geteuid() -> u32;
        fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    }
    const PR_GET_DUMPABLE: i32 = 3;
    const PR_SET_DUMPABLE: i32 = 4;

    /// Serialises every credential-switching dial. A thread's credential change resets the dumpable flag of the
    /// whole process (the kernel sets it on the shared mm to `fs.suid_dumpable`), so two dials running at once could
    /// read each other's reset value, and one dial's restore could make the process dumpable (ptrace-able by the
    /// account) while another dial thread still runs as the account. Held from before the switch until the flag is
    /// restored after the dial thread has exited.
    static DIAL_LOCK: Mutex<()> = Mutex::new(());
    /// The shell's own dumpable flag, read once under [`DIAL_LOCK`] before the first credential switch (so never a
    /// value a dial left behind) and restored after every dial.
    static STARTUP_DUMPABLE: OnceLock<i32> = OnceLock::new();

    fn lock() -> MutexGuard<'static, ()> {
        // the guarded state is the process flag, not data a panic could leave half written
        DIAL_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The flag recorded at the first dial. The caller holds [`DIAL_LOCK`].
    fn startup_dumpable() -> i32 {
        // SAFETY: PR_GET_DUMPABLE takes plain integers.
        *STARTUP_DUMPABLE.get_or_init(|| unsafe { prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) })
    }

    /// What to put back for a recorded flag: `PR_SET_DUMPABLE` takes only 0 and 1, so anything but a recorded 1 (the
    /// root-only 2, or a failed read) restores the safe 0.
    pub(super) fn restore_value(recorded: i32) -> usize {
        if recorded == 1 {
            1
        } else {
            0
        }
    }

    /// Sets the dumpable flag to `want` and checks that it holds.
    fn restore_dumpable(want: usize) -> io::Result<()> {
        // SAFETY: PR_GET_DUMPABLE and PR_SET_DUMPABLE take plain integers.
        unsafe {
            if prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) as i64 != want as i64 {
                prctl(PR_SET_DUMPABLE, want, 0, 0, 0);
            }
            let now = prctl(PR_GET_DUMPABLE, 0, 0, 0, 0);
            if now as i64 != want as i64 {
                return Err(io::Error::other(format!("the shell's dumpable flag is {now} after a dial, not {want}")));
            }
        }
        Ok(())
    }

    /// Switches the calling thread, and only it, to `uid`/`gid` (real, effective and saved, plus the supplementary
    /// groups when it may set them). The raw syscalls are used on purpose: the libc wrappers apply a change to every
    /// thread of the process, which would drop the whole shell's privileges.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) fn become_thread(uid: u32, gid: u32) -> io::Result<()> {
        // SAFETY: plain credential syscalls on the calling thread; `groups` lives across the call.
        unsafe {
            if geteuid() == 0 {
                let groups = [gid];
                if syscall(nr::SETGROUPS, 1i64, groups.as_ptr()) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // The group first: once the uid is dropped the thread may no longer change it.
            if syscall(nr::SETRESGID, gid as i64, gid as i64, gid as i64) != 0 {
                return Err(io::Error::last_os_error());
            }
            if syscall(nr::SETRESUID, uid as i64, uid as i64, uid as i64) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    pub(super) fn become_thread(_uid: u32, _gid: u32) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "per-thread credentials are not wired for this architecture"))
    }

    /// Connects to `path` as `uid`/`gid` from a dedicated thread that ends right after, so the shell's other threads
    /// keep their credentials. The returned stream belongs to the process and is used as usual. Dials are serialised
    /// ([`DIAL_LOCK`]); the dial thread clears the dumpable flag as soon as it has switched (so the account cannot
    /// trace it whatever `fs.suid_dumpable` says), and once it has exited the shell's recorded flag is put back and
    /// checked. The connect does not wait ([`super::connect_now`]), so the lock is held only briefly.
    pub fn connect_as(path: &Path, uid: u32, gid: u32) -> io::Result<UnixStream> {
        let _serial = lock();
        let want = restore_value(startup_dumpable());
        let path = path.to_path_buf();
        let joined = std::thread::Builder::new()
            .name("app-dial".into())
            .spawn(move || {
                become_thread(uid, gid)?;
                // SAFETY: PR_SET_DUMPABLE takes plain integers.
                unsafe { prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) };
                super::connect_now(&path)
            })?
            .join();
        let restored = restore_dumpable(want);
        let stream = joined.map_err(|_| io::Error::other("the dial thread panicked"))??;
        restored?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(shortcut: bool, params: &[&str]) -> ManifestIntent {
        ManifestIntent { id: "new-note".into(), title: "New Note".into(), description: String::new(), shortcut, params: params.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn only_parameterless_shortcuts_are_runnable() {
        assert!(intent(true, &[]).runnable());
        assert!(intent(true, &["due:date?"]).runnable());
        assert!(!intent(true, &["text:string"]).runnable());
        assert!(!intent(false, &[]).runnable());
        let mut bad = intent(true, &[]);
        bad.id = "a\"b".into();
        assert!(!bad.runnable());
    }

    #[test]
    fn runnable_shortcuts_lists_only_parameterless_shortcuts_per_app() {
        use super::super::apphost::{Installed, Manifest};
        let apps = [
            Installed {
                manifest: Manifest::parse("id = \"a.notes\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nshortcut = true\nparams = [\"text:string\"]\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
            Installed {
                manifest: Manifest::parse("id = \"a.quiet\"\nname = \"Quiet\"\nexec = \"x\"\n\n[[intent]]\nid = \"hush\"\ntitle = \"Hush\"\nshortcut = false\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
        ];
        let found = runnable_shortcuts(&apps);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].app_id.as_str(), found[0].app_name.as_str(), found[0].intent.id.as_str()), ("a.notes", "Notes", "new-note"));
        assert!(runnable_shortcuts(&[]).is_empty());
    }

    #[test]
    fn request_and_reply_round_trip() {
        assert_eq!(request_line("new-note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#), Ok(Some("Created".into())));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#), Ok(None));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#), Err("no intent x".into()));
        assert!(parse_reply("nonsense").is_err());
    }

    #[test]
    fn request_lines_carry_string_parameters_escaped() {
        assert_eq!(
            request_line_with("create-note", &[("text", "a \"quote\"\nline\\")]),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\nline\\\\\"}}\n"
        );
        assert_eq!(request_line_with("look-up", &[("word", "cat")]), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\"}}\n");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn run_talks_to_a_socket() {
        let dir = std::env::temp_dir().join(format!("bifrost-intents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("t.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            assert!(req.contains("\"intent.go\""));
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":null,\"dialog\":\"Went\"}}\n").unwrap();
        });
        assert_eq!(run_at(&sock, &request_line("go"), Duration::from_secs(1)), Ok(Some("Went".into())));
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The uid the kernel recorded for the connecting end of `s`.
    #[cfg(target_os = "linux")]
    fn peer_uid(s: &std::os::unix::net::UnixStream) -> u32 {
        use std::os::fd::AsRawFd;
        #[repr(C)]
        struct Ucred {
            pid: i32,
            uid: u32,
            gid: u32,
        }
        extern "C" {
            fn getsockopt(fd: i32, level: i32, name: i32, val: *mut std::ffi::c_void, len: *mut u32) -> i32;
        }
        let mut cred = Ucred { pid: 0, uid: u32::MAX, gid: 0 };
        let mut len = std::mem::size_of::<Ucred>() as u32;
        // SAFETY: cred is a live ucred of the length passed; SOL_SOCKET = 1, SO_PEERCRED = 17.
        assert_eq!(unsafe { getsockopt(s.as_raw_fd(), 1, 17, &mut cred as *mut Ucred as *mut std::ffi::c_void, &mut len) }, 0);
        cred.uid
    }

    #[cfg(target_os = "linux")]
    extern "C" {
        fn geteuid() -> u32;
        fn getegid() -> u32;
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_connects_from_its_own_thread_and_leaves_the_caller_alone() {
        // without root the dial can only take the credentials it already has; the connection is made from the dial
        // thread, the app sees those credentials, and the calling thread (and the process) keep theirs
        let dir = std::env::temp_dir().join(format!("bifrost-intents-dial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("d.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let seen = peer_uid(&s);
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":null,\"dialog\":\"Dialled\"}}\n").unwrap();
            seen
        });
        let mut s = dial(&sock, Some((uid, gid))).unwrap();
        s.write_all(request_line("go").as_bytes()).unwrap();
        let _ = s.shutdown(std::net::Shutdown::Write);
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        assert_eq!(parse_reply(&reply), Ok(Some("Dialled".into())));
        assert_eq!(h.join().unwrap(), uid, "SO_PEERCRED carries the dial thread's uid");
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid), "the caller's credentials are untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_cannot_take_a_foreign_uid_without_privilege() {
        // proves the credential switch really runs: a non-root dial thread may not become another user, so the dial
        // fails with EPERM instead of quietly connecting as itself (skipped as root, where the switch succeeds)
        if unsafe { geteuid() } == 0 {
            return;
        }
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let err = dial(std::path::Path::new("/nonexistent/bifrost-dial.sock"), Some((uid + 1, gid))).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
        assert_eq!(unsafe { geteuid() }, uid);
    }

    #[test]
    fn without_an_account_the_socket_is_dialled_directly() {
        assert!(dial(std::path::Path::new("/nonexistent/bifrost-dial.sock"), None).is_err());
    }

    #[test]
    fn the_socket_owner_picks_the_dial_credentials() {
        let account = Some((1000, 1000));
        // a sandboxed app: the account owns its data folder and socket
        assert_eq!(dial_identity(Some(1000), Some(1000), account), account);
        // an app the shell started as root (no sandbox): a root-only socket in a root folder is dialled as root
        assert_eq!(dial_identity(Some(0), Some(0), account), None);
        // a root-owned socket in the account's folder may have been swapped in by the app: least privilege
        assert_eq!(dial_identity(Some(0), Some(1000), account), account);
        // missing or unreadable socket, or a foreign owner: as the account (the app refuses what it must)
        assert_eq!(dial_identity(None, Some(0), account), account);
        assert_eq!(dial_identity(Some(1001), Some(1001), account), account);
        // no account (the shell is not root, or before setup): always as the shell itself
        assert_eq!(dial_identity(Some(1000), Some(1000), None), None);
        assert_eq!(dial_identity(Some(0), Some(0), None), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_dumpable_flag_restores_only_what_prctl_accepts() {
        assert_eq!(account_dial::restore_value(1), 1);
        assert_eq!(account_dial::restore_value(0), 0);
        assert_eq!(account_dial::restore_value(2), 0, "the root-only 2 a credential change leaves is never restored");
        assert_eq!(account_dial::restore_value(-1), 0, "a failed read restores the safe value");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_socket_that_never_accepts_fails_the_dial_instead_of_blocking() {
        use std::os::fd::AsRawFd;
        extern "C" {
            fn listen(fd: i32, backlog: i32) -> i32;
        }
        let dir = std::env::temp_dir().join(format!("bifrost-intents-full-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("f.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // a backlog of 0 holds one pending connection; the app never accepts it
        // SAFETY: a live listening socket; listen again only changes its backlog
        assert_eq!(unsafe { listen(l.as_raw_fd(), 0) }, 0);
        let _pending = UnixStream::connect(&sock).unwrap();
        let err = dial(&sock, None).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock, "{err}");
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let err = dial(&sock, Some((uid, gid))).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock, "the credential dial does not wait either: {err}");
        let started = Instant::now();
        assert!(run_at(&sock, &request_line("go"), Duration::from_millis(300)).is_err());
        assert!(started.elapsed() < Duration::from_secs(3), "the intent fails once its wait is over");
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn concurrent_account_dials_leave_the_dumpable_flag_as_it_was() {
        extern "C" {
            fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
        }
        // SAFETY: PR_GET_DUMPABLE (3) takes plain integers.
        let before = unsafe { prctl(3, 0, 0, 0, 0) };
        let dir = std::env::temp_dir().join(format!("bifrost-intents-many-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("m.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        const DIALS: usize = 8;
        let server = std::thread::spawn(move || {
            for _ in 0..DIALS {
                let _ = l.accept().unwrap();
            }
        });
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let dialers: Vec<_> = (0..DIALS)
            .map(|_| {
                let sock = sock.clone();
                std::thread::spawn(move || dial(&sock, Some((uid, gid))).map(|_| ()))
            })
            .collect();
        for d in dialers {
            d.join().unwrap().unwrap();
        }
        server.join().unwrap();
        // SAFETY: as above
        assert_eq!(unsafe { prctl(3, 0, 0, 0, 0) }, account_dial::restore_value(before) as i32, "every dial restores the recorded flag");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shell_dials_the_control_socket_in_the_app_data_directory() {
        // the sandboxed app has no host runtime directory: it binds `control.sock` in BIFROST_DATA, which the launcher
        // sets to `<root>/appdata/<id>` and binds at that same path
        let id = "is.olibuijr.dictionary.desktop";
        let data = super::super::apphost::root().join("appdata").join(id);
        assert_eq!(socket_path(id), data.join("control.sock"));
        assert_eq!(socket_path(id), bifrost_core::app_control_socket::socket_path(None, &data), "the SDK runtime's rule");
        assert!(!socket_path(id).to_string_lossy().contains("bifrost/apps"));
    }

    #[test]
    fn only_apps_that_declare_the_visual_search_intent_are_asked() {
        use super::super::apphost::{Installed, Manifest};
        let apps = [
            Installed {
                manifest: Manifest::parse("id = \"a.plants\"\nname = \"Plants\"\nexec = \"x\"\n\n[[intent]]\nid = \"visual-search\"\ntitle = \"Visual Search\"\nshortcut = false\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
            Installed {
                manifest: Manifest::parse("id = \"a.notes\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
        ];
        assert_eq!(visual_search_apps(&apps), vec!["a.plants".to_string()]);
        assert!(visual_search_apps(&[]).is_empty());
        let scene = bifrost_ui::visual_search::Scene::new(bifrost_ui::visual_search::SceneKind::Screenshot, 1, 1, vec![0, 0, 0, 255]).unwrap();
        assert!(visual_search(&apps, &scene).is_empty(), "an app that is not running is skipped, not started");
    }

    #[test]
    fn a_visual_search_goes_to_the_app_socket_and_the_matches_are_stamped_with_that_app() {
        use bifrost_ui::visual_search::{Scene, SceneKind};
        let dir = std::env::temp_dir().join(format!("bifrost-visual-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("p.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            assert!(req.contains("\"visual_search.search\""));
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{\"intent\":\"open\",\"title\":\"Fern\",\"score\":70}]}\n").unwrap();
        });
        let scene = Scene::new(SceneKind::Screenshot, 2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap().with_labels(&["plant"]);
        let found = visual_search_at(&sock, "is.x.plants", &scene, Duration::from_secs(1));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].app.as_str(), found[0].title.as_str(), found[0].score), ("is.x.plants", "Fern", 70));
        h.join().unwrap();
        assert!(visual_search_at(&dir.join("none.sock"), "is.x.gone", &scene, Duration::ZERO).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
