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
    let Ok(reply) = exchange_within(path, &line, wait, VISUAL_SEARCH_REPLY_WAIT) else { return Vec::new() };
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

/// Largest reply the shell reads from an app, in bytes. A longer one fails the call instead of filling the shell's memory.
pub const MAX_REPLY: u64 = 1024 * 1024;
/// How long the shell gives an app to take the request and answer it, once connected: the SDK's longest pending call
/// (`bifrost_core::app_control_socket::MAX_PENDING_MS`, `bifrost_app::control::MAX_TIMEOUT_MS`) plus a margin for the
/// app's event loop, so a slow async answer arrives before the shell gives up.
pub const REPLY_WAIT: Duration = Duration::from_millis(bifrost_core::app_control_socket::MAX_PENDING_MS + 2_000);
/// The reply wait of one visual search provider. The providers are asked one after another, so one that never
/// answers must not hold the whole search for [`REPLY_WAIT`].
const VISUAL_SEARCH_REPLY_WAIT: Duration = Duration::from_secs(5);

fn exchange_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<String, String> {
    exchange_within(path, line, wait, REPLY_WAIT)
}

/// One JSON-RPC exchange: connects (retrying while the app starts, up to `wait`), then writes `line` and reads the
/// reply, all of it within `reply_wait` of the connect and at most [`MAX_REPLY`] bytes.
fn exchange_within(path: &std::path::Path, line: &str, wait: Duration, reply_wait: Duration) -> Result<String, String> {
    let deadline = Instant::now() + wait;
    let caller = caller();
    let stream = loop {
        // the socket is opened again on every attempt: it appears (or is replaced) while the app starts
        match dial(path, caller) {
            Ok(s) => break s,
            Err(e) if Instant::now() >= deadline => return Err(format!("{}: {e}", path.display())),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let deadline = Instant::now() + reply_wait;
    write_by(&stream, line.as_bytes(), deadline)?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    read_reply(&stream, deadline)
}

/// The time left before `deadline`, or an error once it has passed.
fn time_left(deadline: Instant) -> Result<Duration, String> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err("the app did not answer in time".into());
    }
    Ok(left)
}

/// Writes all of `buf`, giving up at `deadline` however the app paces its reads.
fn write_by(stream: &UnixStream, mut buf: &[u8], deadline: Instant) -> Result<(), String> {
    let mut w = stream;
    while !buf.is_empty() {
        stream.set_write_timeout(Some(time_left(deadline)?)).map_err(|e| e.to_string())?;
        match w.write(buf) {
            Ok(0) => return Err("the app closed the connection before taking the request".into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// Reads the reply up to end of stream: at most [`MAX_REPLY`] bytes and only until `deadline` (checked before every
/// read, and each read waits no longer than what is left), so an app that trickles bytes or streams without end
/// fails the call instead of holding the caller or exhausting the shell's memory.
fn read_reply(stream: &UnixStream, deadline: Instant) -> Result<String, String> {
    // one byte past the limit tells a reply of exactly MAX_REPLY bytes from a longer one
    let mut limited = Read::take(stream, MAX_REPLY + 1);
    let mut reply = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        stream.set_read_timeout(Some(time_left(deadline)?)).map_err(|e| e.to_string())?;
        match limited.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
        if reply.len() as u64 > MAX_REPLY {
            return Err(format!("the app's reply is longer than {MAX_REPLY} bytes"));
        }
    }
    String::from_utf8(reply).map_err(|_| "the app's reply is not UTF-8".to_string())
}

/// Who the shell is when it dials: an ordinary user (its effective uid), or root with the installed account's
/// `(uid, gid)` (None before setup, or when the account cannot be read).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caller {
    User(u32),
    Root(Option<(u32, u32)>),
}

/// The credentials a dial uses: the shell's own, or the account's (`(uid, gid)`, switched in for the connect only).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialAs {
    Itself,
    Account(u32, u32),
}

/// The shell as it dials now.
#[cfg(target_os = "linux")]
fn caller() -> Caller {
    if crate::appearance_portal::is_root() {
        Caller::Root(super::screenshot::account_owner())
    } else {
        extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: geteuid has no preconditions.
        Caller::User(unsafe { geteuid() })
    }
}

#[cfg(not(target_os = "linux"))]
fn caller() -> Caller {
    Caller::User(0)
}

/// Whose credentials dial a control socket whose inode belongs to `socket_owner`, inside the app folder owned by
/// `dir_owner` (both read from the opened descriptors, never through a link). The socket and its folder must have one
/// owner, and that owner decides:
/// - a shell running as an ordinary user dials only its own sockets, as itself;
/// - a root shell dials a root socket in a root folder (an app the shell started as root, without the sandbox) as
///   root, and the account's socket in the account's folder (a sandboxed app, whose user namespace maps only the
///   account's uid, so root would arrive as the overflow uid and be refused) as the account: least privilege;
/// - everything else is refused. Above all a root shell with no account (before setup, or when the account cannot be
///   read) never dials a socket that is not root's as root, so an app cannot turn a root dial on another socket.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn dial_identity(socket_owner: u32, dir_owner: u32, caller: Caller) -> Result<DialAs, String> {
    if socket_owner != dir_owner {
        return Err(format!("the control socket (uid {socket_owner}) and its folder (uid {dir_owner}) have different owners"));
    }
    match caller {
        Caller::User(me) if socket_owner == me => Ok(DialAs::Itself),
        Caller::Root(_) if socket_owner == 0 => Ok(DialAs::Itself),
        Caller::Root(Some((uid, gid))) if socket_owner == uid => Ok(DialAs::Account(uid, gid)),
        _ => Err(format!("the control socket belongs to uid {socket_owner}, which the shell does not dial")),
    }
}

/// Connects to an app's control socket. The socket is pinned first ([`pinned::open`]: its folder and the socket are
/// opened without following a link, and their owners read from the descriptors), [`dial_identity`] picks the
/// credentials from those owners, and the connect goes to the pinned inode through `/proc/self/fd/<n>`, so nothing
/// the app does between the check and the connect changes what is dialled. The connect never waits: an app that does
/// not accept (its backlog full) fails the attempt with `WouldBlock` at once, and [`exchange_at`] retries until its
/// deadline, so a dial is bounded and never holds the credential lock for long.
#[cfg(target_os = "linux")]
fn dial(path: &std::path::Path, caller: Caller) -> std::io::Result<UnixStream> {
    let socket = pinned::open(path)?;
    let how = dial_identity(socket.socket_owner, socket.dir_owner, caller).map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
    // `socket` stays open until the connect returns: the fd path names its descriptor
    let via = socket.fd_path();
    match how {
        DialAs::Itself => connect_now(&via),
        DialAs::Account(uid, gid) => account_dial::connect_as(&via, uid, gid),
    }
}

#[cfg(not(target_os = "linux"))]
fn dial(path: &std::path::Path, _caller: Caller) -> std::io::Result<UnixStream> {
    connect_now(path)
}

/// A control socket opened without following links.
#[cfg(target_os = "linux")]
mod pinned {
    use std::ffi::{c_char, c_int, CString};
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::path::{Path, PathBuf};

    const O_CLOEXEC: c_int = 0o2000000;
    const O_PATH: c_int = 0o10000000;
    // the directory and no-follow bits differ on the ARM ABIs
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    const O_DIRECTORY: c_int = 0o40000;
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    const O_NOFOLLOW: c_int = 0o100000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
    const O_DIRECTORY: c_int = 0o200000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
    const O_NOFOLLOW: c_int = 0o400000;
    const AT_FDCWD: c_int = -100;

    extern "C" {
        fn openat(dirfd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    }

    /// The socket inode, held by an `O_PATH` descriptor, with the owners of the socket and of its folder.
    pub(super) struct Socket {
        fd: File,
        pub(super) socket_owner: u32,
        pub(super) dir_owner: u32,
    }

    impl Socket {
        /// The path that names the pinned inode while `self` lives (`connect` follows this one magic link to it).
        pub(super) fn fd_path(&self) -> PathBuf {
            PathBuf::from(format!("/proc/self/fd/{}", self.fd.as_raw_fd()))
        }
    }

    fn open_path(dirfd: c_int, name: &[u8], flags: c_int) -> io::Result<File> {
        let c = CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "control socket path holds a NUL"))?;
        // SAFETY: `c` is NUL-terminated and lives across the call; O_PATH opens take no mode argument.
        let fd = unsafe { openat(dirfd, c.as_ptr(), flags | O_PATH | O_NOFOLLOW | O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the kernel returned a new descriptor that nothing else owns.
        Ok(File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// Opens `path`'s folder (the app's data folder; its own last component is not followed) and then the socket in
    /// it by descriptor, without following a link either. A link in either place, anything but a folder and a socket,
    /// or a lookup that would leave the folder (the name is one plain component) is refused. Only `O_PATH`
    /// descriptors are taken, so opening reads nothing and talks to no file system driver beyond the lookup.
    pub(super) fn open(path: &Path) -> io::Result<Socket> {
        let refuse = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: {what}", path.display()));
        let name = path.file_name().ok_or_else(|| refuse("no socket name"))?.as_bytes();
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.as_os_str().as_bytes(),
            _ => &b"."[..],
        };
        let dir = open_path(AT_FDCWD, dir, O_DIRECTORY)?;
        let dir_meta = dir.metadata()?;
        if !dir_meta.is_dir() {
            return Err(refuse("the app folder is a link or not a folder"));
        }
        let fd = open_path(dir.as_raw_fd(), name, 0)?;
        let meta = fd.metadata()?;
        if !meta.file_type().is_socket() {
            return Err(refuse("not a socket (a link is not followed)"));
        }
        Ok(Socket { fd, socket_owner: meta.uid(), dir_owner: dir_meta.uid() })
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
    use crate::bare::sys::{connect, socket};

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
    if unsafe { connect(owned.as_raw_fd(), (&addr as *const SockaddrUn).cast(), len) } != 0 {
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

/// The account-credential connect: the calling thread switches only its effective ids to the account's for the
/// connect, and back right after.
///
/// Why only the effective ids: `SO_PEERCRED` records the connecting thread's effective uid and gid, so the app sees
/// the account. The real and saved uid stay 0 throughout, so the account's processes match neither the signal rule
/// (sender uid against the target's real or saved uid) nor the ptrace rule (the caller's ids against all of the
/// target's) for this thread at any moment, and no thread ever exits with the account's credentials. While the
/// effective uid is not 0 the thread's effective capabilities are dropped, so the socket's permission is checked as
/// the account; back at effective uid 0 they return from the permitted set, which the switch never touched.
#[cfg(target_os = "linux")]
mod account_dial {
    use std::io;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    #[cfg(target_arch = "x86_64")]
    mod nr {
        pub const GETGROUPS: i64 = 115;
        pub const SETGROUPS: i64 = 116;
        pub const SETRESUID: i64 = 117;
        pub const SETRESGID: i64 = 119;
    }
    #[cfg(target_arch = "aarch64")]
    mod nr {
        pub const GETGROUPS: i64 = 158;
        pub const SETGROUPS: i64 = 159;
        pub const SETRESUID: i64 = 147;
        pub const SETRESGID: i64 = 149;
    }

    extern "C" {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn syscall(n: i64, ...) -> i64;
        fn geteuid() -> u32;
        fn getegid() -> u32;
        fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    }
    const PR_GET_DUMPABLE: i32 = 3;
    const PR_SET_DUMPABLE: i32 = 4;
    /// "Leave this id as it is" for setresuid and setresgid.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    const KEEP: i64 = -1;

    /// Serialises every credential-switching dial. Any change of a thread's effective ids resets the dumpable flag of
    /// the whole process (`commit_creds` sets it on the shared mm to `fs.suid_dumpable`), so two dials running at once
    /// could read each other's reset value as the one to restore. Held from before the switch until the flag is put
    /// back after the switch back.
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

    /// Sets the dumpable flag to `want`, checking every prctl return, and checks that it holds.
    fn restore_dumpable(want: usize) -> io::Result<()> {
        // SAFETY: PR_GET_DUMPABLE and PR_SET_DUMPABLE take plain integers.
        unsafe {
            let now = prctl(PR_GET_DUMPABLE, 0, 0, 0, 0);
            if now < 0 {
                return Err(io::Error::last_os_error());
            }
            if now as usize != want && prctl(PR_SET_DUMPABLE, want, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            let now = prctl(PR_GET_DUMPABLE, 0, 0, 0, 0);
            if now < 0 || now as usize != want {
                return Err(io::Error::other(format!("the shell's dumpable flag is {now} after a dial, not {want}")));
            }
        }
        Ok(())
    }

    /// The calling thread's supplementary groups.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn thread_groups() -> io::Result<Vec<u32>> {
        // SAFETY: a size of 0 only asks for the count and writes nothing.
        let n = unsafe { syscall(nr::GETGROUPS, 0i64, std::ptr::null_mut::<u32>()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut groups = vec![0u32; n as usize];
        // SAFETY: `groups` holds `n` writable u32s. Only this thread changes its own groups, so the count holds.
        let got = unsafe { syscall(nr::GETGROUPS, n, groups.as_mut_ptr()) };
        if got < 0 {
            return Err(io::Error::last_os_error());
        }
        groups.truncate(got as usize);
        Ok(groups)
    }

    /// The calling thread's effective ids (and, for root, its groups) from before a switch. Dropping it switches the
    /// thread back; a thread that cannot be switched back aborts the process, which must never go on with a thread
    /// running as the account.
    pub(super) struct Switched {
        euid: u32,
        egid: u32,
        groups: Option<Vec<u32>>,
    }

    impl Switched {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn restore(&self) -> io::Result<()> {
            // SAFETY: plain credential syscalls on the calling thread; `groups` lives across the call.
            unsafe {
                // the uid first: back at effective uid 0 the thread may set its gid and groups again. Going back is
                // allowed without privilege, as the real and saved ids never changed.
                if syscall(nr::SETRESUID, KEEP, self.euid as i64, KEEP) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if syscall(nr::SETRESGID, KEEP, self.egid as i64, KEEP) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if let Some(groups) = &self.groups {
                    if syscall(nr::SETGROUPS, groups.len() as i64, groups.as_ptr()) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                if geteuid() != self.euid || getegid() != self.egid {
                    return Err(io::Error::other("the dial thread's effective ids did not switch back"));
                }
            }
            Ok(())
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        fn restore(&self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for Switched {
        fn drop(&mut self) {
            if let Err(e) = self.restore() {
                eprintln!("springboard: cannot switch the dial thread back to uid {} gid {}: {e}; aborting", self.euid, self.egid);
                std::process::abort();
            }
        }
    }

    /// Switches the calling thread's effective ids, and only those, to `uid`/`gid`, plus the supplementary groups (just
    /// `gid`) when the thread is root. The raw syscalls are used on purpose: the libc wrappers apply a change to every
    /// thread of the process, which would hand the whole shell to the account. Every return is checked; a failure
    /// partway switches back what changed (the returned guard's drop does it on every path).
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) fn switch_to(uid: u32, gid: u32) -> io::Result<Switched> {
        // SAFETY: geteuid and getegid have no preconditions.
        let (euid, egid) = unsafe { (geteuid(), getegid()) };
        let groups = if euid == 0 { Some(thread_groups()?) } else { None };
        // made before the first change, so an error below switches back whatever already changed
        let back = Switched { euid, egid, groups };
        // SAFETY: plain credential syscalls on the calling thread; `account_groups` lives across the call.
        unsafe {
            if back.groups.is_some() {
                let account_groups = [gid];
                if syscall(nr::SETGROUPS, 1i64, account_groups.as_ptr()) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // the group first: once the effective uid is the account's the thread may no longer change it
            if syscall(nr::SETRESGID, KEEP, gid as i64, KEEP) != 0 {
                return Err(io::Error::last_os_error());
            }
            if syscall(nr::SETRESUID, KEEP, uid as i64, KEEP) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(back)
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    pub(super) fn switch_to(_uid: u32, _gid: u32) -> io::Result<Switched> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "per-thread credentials are not wired for this architecture"))
    }

    /// Connects to `path` with the account's effective ids on the calling thread, which switches back before this
    /// returns, so the shell's other threads never change and no thread is spawned. Dials are serialised
    /// ([`DIAL_LOCK`]); after the switch back the shell's recorded dumpable flag is put back and checked, and a
    /// failure to restore it is logged even when the dial failed too. The connect does not wait
    /// ([`super::connect_now`]), so the lock is held only briefly.
    pub fn connect_as(path: &Path, uid: u32, gid: u32) -> io::Result<UnixStream> {
        let _serial = lock();
        let want = restore_value(startup_dumpable());
        let dialled = switch_to(uid, gid).and_then(|back| {
            let stream = super::connect_now(path);
            drop(back);
            stream
        });
        let restored = restore_dumpable(want);
        if let Err(e) = &restored {
            eprintln!("springboard: control socket dial: {e}");
        }
        let stream = dialled?;
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

    /// A listening socket in a fresh folder of its own, named for the test.
    #[cfg(target_os = "linux")]
    fn listener(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::os::unix::net::UnixListener) {
        let dir = std::env::temp_dir().join(format!("bifrost-intents-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        (dir, sock, l)
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_connects_on_the_calling_thread_and_switches_back() {
        // without root the dial can only take the effective ids it already has; the connect is made on the calling
        // thread, the app sees those ids, and the thread has its own back once the dial returns
        let (dir, sock, l) = listener("dial");
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let seen = peer_uid(&s);
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":null,\"dialog\":\"Dialled\"}}\n").unwrap();
            seen
        });
        let s = account_dial::connect_as(&sock, uid, gid).unwrap();
        write_by(&s, request_line("go").as_bytes(), Instant::now() + Duration::from_secs(2)).unwrap();
        let _ = s.shutdown(std::net::Shutdown::Write);
        let reply = read_reply(&s, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(parse_reply(&reply), Ok(Some("Dialled".into())));
        assert_eq!(h.join().unwrap(), uid, "SO_PEERCRED carries the dialling thread's effective uid");
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid), "the calling thread is switched back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_pinned_dial_reaches_the_socket_as_the_shell() {
        // the dial goes through the descriptor of the socket it checked (`/proc/self/fd/<n>`), not the path
        let (dir, sock, l) = listener("pinned");
        let uid = unsafe { geteuid() };
        let h = std::thread::spawn(move || peer_uid(&l.accept().unwrap().0));
        dial(&sock, caller()).unwrap();
        assert_eq!(h.join().unwrap(), uid);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_link_is_never_followed_to_a_socket() {
        // an app that swaps its socket, or its folder, for a link to another socket reaches nothing
        let (dir, sock, l) = listener("link");
        l.set_nonblocking(true).unwrap();
        let link = dir.join("link.sock");
        std::os::unix::fs::symlink(&sock, &link).unwrap();
        let err = dial(&link, caller()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
        let folder_link = dir.with_extension("link");
        let _ = std::fs::remove_file(&folder_link);
        std::os::unix::fs::symlink(&dir, &folder_link).unwrap();
        assert!(dial(&folder_link.join("control.sock"), caller()).is_err(), "the app folder itself is not followed either");
        assert_eq!(l.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock, "no dial reached the real socket");
        // not a socket at all
        std::fs::write(dir.join("plain"), b"x").unwrap();
        assert_eq!(dial(&dir.join("plain"), caller()).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_file(&folder_link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn the_credential_switch_checks_every_step_and_puts_back_what_it_changed() {
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        // the ids the thread already has: the switch succeeds and dropping the guard switches back
        let back = account_dial::switch_to(uid, gid).unwrap();
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid));
        drop(back);
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid));
        if uid == 0 {
            return; // root may take any ids; the refusals below need an unprivileged thread
        }
        // a foreign gid or uid is refused without privilege, and the thread keeps its ids
        assert_eq!(account_dial::switch_to(uid, gid.wrapping_add(1)).err().map(|e| e.kind()), Some(std::io::ErrorKind::PermissionDenied));
        assert_eq!(account_dial::switch_to(uid.wrapping_add(1), gid).err().map(|e| e.kind()), Some(std::io::ErrorKind::PermissionDenied));
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_cannot_take_a_foreign_uid_without_privilege() {
        // proves the credential switch really runs: a non-root thread may not become another user, so the dial
        // fails with EPERM instead of quietly connecting as itself (skipped as root, where the switch succeeds)
        if unsafe { geteuid() } == 0 {
            return;
        }
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let err = account_dial::connect_as(std::path::Path::new("/nonexistent/bifrost-dial.sock"), uid + 1, gid).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid));
    }

    #[test]
    fn a_missing_socket_fails_the_dial() {
        assert!(dial(std::path::Path::new("/nonexistent/bifrost-dial.sock"), caller()).is_err());
    }

    #[test]
    fn the_socket_owner_picks_the_dial_credentials() {
        let account = Caller::Root(Some((1000, 1000)));
        // a sandboxed app: the account owns its data folder and socket, dialled as the account
        assert_eq!(dial_identity(1000, 1000, account), Ok(DialAs::Account(1000, 1000)));
        // an app the shell started as root (no sandbox): a root socket in a root folder is dialled as root
        assert_eq!(dial_identity(0, 0, account), Ok(DialAs::Itself));
        // socket and folder disagree (a socket swapped in by the app, or put there by someone else): refused
        assert!(dial_identity(0, 1000, account).is_err());
        assert!(dial_identity(1000, 0, account).is_err());
        // a foreign owner is refused, whoever the shell is
        assert!(dial_identity(1001, 1001, account).is_err());
        // a root shell with no account (before setup, or the account unreadable) dials only root's sockets as root
        // and never falls back to a root dial of anything else
        assert_eq!(dial_identity(0, 0, Caller::Root(None)), Ok(DialAs::Itself));
        assert!(dial_identity(1000, 1000, Caller::Root(None)).is_err());
        // a shell running as a user dials only its own sockets, as itself
        assert_eq!(dial_identity(1000, 1000, Caller::User(1000)), Ok(DialAs::Itself));
        assert!(dial_identity(0, 0, Caller::User(1000)).is_err());
        assert!(dial_identity(1001, 1001, Caller::User(1000)).is_err());
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
        let (dir, sock, l) = listener("full");
        // a backlog of 0 holds one pending connection; the app never accepts it
        // SAFETY: a live listening socket; listen again only changes its backlog
        assert_eq!(unsafe { listen(l.as_raw_fd(), 0) }, 0);
        let _pending = UnixStream::connect(&sock).unwrap();
        let err = dial(&sock, caller()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock, "{err}");
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let err = account_dial::connect_as(&sock, uid, gid).unwrap_err();
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
        let (dir, sock, l) = listener("many");
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
                std::thread::spawn(move || account_dial::connect_as(&sock, uid, gid).map(|_| ()))
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
    fn a_reply_is_read_up_to_the_limit_and_no_further() {
        let (a, b) = UnixStream::pair().unwrap();
        let w = std::thread::spawn(move || {
            let mut b = b;
            b.write_all(&vec![b'a'; MAX_REPLY as usize]).unwrap();
        });
        let reply = read_reply(&a, Instant::now() + Duration::from_secs(10)).unwrap();
        assert_eq!(reply.len() as u64, MAX_REPLY, "a reply of exactly the limit is whole");
        w.join().unwrap();

        let (a, b) = UnixStream::pair().unwrap();
        let w = std::thread::spawn(move || {
            let mut b = b;
            // the reader stops after the limit; the rest fails to send once it hangs up
            let _ = b.write_all(&vec![b'a'; MAX_REPLY as usize + 4096]);
        });
        let err = read_reply(&a, Instant::now() + Duration::from_secs(10)).unwrap_err();
        assert!(err.contains("longer than"), "{err}");
        drop(a);
        w.join().unwrap();
    }

    #[test]
    fn a_trickling_reply_fails_at_the_deadline() {
        // one byte now and then never trips a per-read timeout; the overall deadline still ends the call
        let (a, b) = UnixStream::pair().unwrap();
        let w = std::thread::spawn(move || {
            let mut b = b;
            while b.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let started = Instant::now();
        let err = read_reply(&a, Instant::now() + Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("in time"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(a);
        w.join().unwrap();
        assert!(time_left(Instant::now()).is_err(), "a deadline that has passed leaves no time");
    }

    #[test]
    fn the_shell_waits_for_the_longest_pending_reply_an_app_may_hold() {
        assert!(REPLY_WAIT > Duration::from_millis(bifrost_core::app_control_socket::MAX_PENDING_MS));
        assert!(VISUAL_SEARCH_REPLY_WAIT < REPLY_WAIT, "one visual search provider cannot hold the whole search");
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
