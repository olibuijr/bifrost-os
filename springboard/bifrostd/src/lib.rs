//! `bifrostd`: keeps the `kind = "service"` packages available, started on demand (docs/services.md,
//! docs/boot-architecture.md "Services on demand").
//!
//! Services live like apps, under `<root>/apps/<id>/current/` with a `manifest.toml`, and the system's own services
//! ship with the OS package under `/usr/lib/bifrost/services/<id>/` (read-only; a store-installed copy wins only when
//! its manifest version is newer, see [`scan_with`]). bifrostd binds each service's
//! socket (`<root>/run/<id>.sock`) itself, and starts the service's child only when a client is waiting on that
//! socket (socket activation). The waiting connection sits in the listen backlog until the child accepts it: the
//! child gets the listening socket as its stdin and `BIFROST_ACTIVATED=1` (`bifrost_proto::svc::listen`). A child
//! that exits is restarted on the next waiting connection, with backoff, and never given up on. A new `current` is
//! swapped in only after it answers a ping on a side socket; else the old version keeps running and the new one is
//! not retried until `current` changes again.
//!
//! `bifrostd desktop-ready` (run by `bifrost-desktop-ready.service`) starts `bifrost-after-desktop.target` once the
//! first desktop frame is on screen, so the services that are not needed for the desktop start after it.

use bifrost_proto::svc::{self, common};
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Swap and memory pressure (`dynamic_pager` counterpart): zram at boot, swap files under pressure, the level file.
pub mod pager;
/// Purgeable storage (`deleted_helper` counterpart): low-space purge, the Free Up Space request, the Purgeable figure.
pub mod purge;

pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A run this long counts as healthy and resets the backoff.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
pub const STOP_GRACE: Duration = Duration::from_secs(3);
/// How often the supervisor checks its children when nothing is waiting on a socket.
pub const TICK: Duration = Duration::from_secs(2);
/// The boot milestone log the shell and bootweave write (`<ms> <event>` lines).
pub const BOOT_MARKS: &str = "/run/bifrost/boot-marks";
/// The mark for the first complete desktop frame (wallpaper, menu bar and Dock presented).
pub const DESKTOP_READY_MARK: &str = "desktop-ready";
/// How long `desktop-ready` waits for the mark before starting the target anyway (the reveal's own ceiling is 4 s).
pub const DESKTOP_READY_WAIT: Duration = Duration::from_secs(6);
pub const AFTER_DESKTOP_TARGET: &str = "bifrost-after-desktop.target";

/// What the supervisor needs from a service's `manifest.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub exec: String,
    /// who the service runs as: empty or `root` (the default), `session` (the logged-in desktop user), or a system
    /// user name. A service that decodes untrusted data (the media player) is `session`, never root.
    pub user: String,
}

impl Manifest {
    /// The same TOML subset the shell reads (`key = "string"`); None unless it is a service with a safe id and exec.
    pub fn parse(text: &str) -> Option<Manifest> {
        let mut m = Manifest::default();
        for line in text.lines() {
            let Some((k, v)) = line.trim().split_once('=') else { continue };
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "id" => m.id = v,
                "version" => m.version = v,
                "kind" => m.kind = v,
                "exec" => m.exec = v,
                "user" => m.user = v,
                _ => {}
            }
        }
        let safe_id = !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        let safe_exec = !m.exec.is_empty() && !m.exec.starts_with('/') && !m.exec.split('/').any(|p| p == "..");
        let safe_user = m.user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        // only `service` packages: the shell's kind table (src/shell/kinds.rs) is the reference
        (m.kind == "service" && safe_id && safe_exec && safe_user).then_some(m)
    }
}

/// A service found under `<root>/apps/<id>/current/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub manifest: Manifest,
    /// the resolved version directory `current` points at
    pub dir: PathBuf,
}

impl Found {
    pub fn exec_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.exec)
    }
}

/// True when `exec` under `dir` is a regular file whose resolved path stays inside `dir` (a symlink must not
/// point a service at a binary elsewhere on the system).
pub fn exec_inside(dir: &Path, exec: &str) -> bool {
    let p = dir.join(exec);
    p.is_file() && std::fs::canonicalize(&p).is_ok_and(|r| r.starts_with(dir))
}

/// Where the OS package puts the system services: `<dir>/<id>/manifest.toml` next to the executable. The package owns
/// the directory (a path no earlier install has a file at), so an update replaces it without a pacman conflict.
pub const SYSTEM_SERVICES_DIR: &str = "/usr/lib/bifrost/services";

/// The dotted numbers of a version, one per component; a component keeps its leading digits (`3-rc1` is 3) and a
/// component with none is 0. `0.10.8` is `[0, 10, 8]`.
fn version_parts(v: &str) -> Vec<u64> {
    v.trim().split('.').map(|c| c.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect()
}

/// Whether version `a` is strictly newer than `b` (numeric, component by component; missing components are 0).
pub fn version_newer(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (version_parts(a), version_parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// The services installed from the store: `<root>/apps/<id>/current/manifest.toml`.
fn scan_store(root: &Path) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(root.join("apps")) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let cur = e.path().join("current");
            let dir = std::fs::canonicalize(&cur).ok()?;
            let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("manifest.toml")).ok()?)?;
            (manifest.id == e.file_name().to_string_lossy() && exec_inside(&dir, &manifest.exec)).then_some(Found { manifest, dir })
        })
        .collect()
}

/// The services the OS package provides: `<system>/<id>/manifest.toml` (the directory itself, no `current` link).
fn scan_system(system: &Path) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(system) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let dir = std::fs::canonicalize(e.path()).ok()?;
            let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("manifest.toml")).ok()?)?;
            (manifest.id == e.file_name().to_string_lossy() && exec_inside(&dir, &manifest.exec)).then_some(Found { manifest, dir })
        })
        .collect()
}

/// The store-installed services only (the package-provided ones are in [`scan_with`]).
pub fn scan(root: &Path) -> Vec<Found> {
    scan_with(root, Path::new("/nonexistent"))
}

/// Every service to supervise: the store-installed ones under `<root>/apps` and the package-provided ones under
/// `system`. When both name the same id the store copy wins only if its manifest version is newer than the package's,
/// so an update of the OS never loses to a stale store install, and a store update still takes effect. Sorted by id.
pub fn scan_with(root: &Path, system: &Path) -> Vec<Found> {
    let mut v = scan_system(system);
    for s in scan_store(root) {
        match v.iter_mut().find(|f| f.manifest.id == s.manifest.id) {
            Some(sys) => {
                if version_newer(&s.manifest.version, &sys.manifest.version) {
                    *sys = s;
                }
            }
            None => v.push(s),
        }
    }
    v.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    v
}

/// Restart delay: 1 s, doubling to 30 s; a stable run starts over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// Records a run that lasted `ran` and returns the delay before the next start.
    pub fn failed(&mut self, ran: Duration) -> Duration {
        if ran >= STABLE_AFTER {
            self.failures = 0;
        }
        let d = BACKOFF_FIRST.saturating_mul(1u32 << self.failures.min(5));
        self.failures = self.failures.saturating_add(1);
        d.min(BACKOFF_MAX)
    }
}

/// Game Mode's priority socket (`priority.rs`): `<root>/run/priority.sock`.
pub const PRIORITY_SOCKET: &str = "priority.sock";

pub mod priority;

pub fn run_dir(root: &Path) -> PathBuf {
    root.join("run")
}

pub fn sock_path(root: &Path, id: &str) -> PathBuf {
    run_dir(root).join(format!("{id}.sock"))
}

/// Pings the service on `sock`; true when it answers as `id`.
pub fn healthy(sock: &Path, id: &str) -> bool {
    let attempt = || -> io::Result<bool> {
        let mut s = UnixStream::connect(sock)?;
        s.set_read_timeout(Some(Duration::from_secs(1)))?;
        s.set_write_timeout(Some(Duration::from_secs(1)))?;
        svc::write_frame(&mut s, &common::ping())?;
        let reply = svc::read_frame(&mut s)?;
        Ok(common::parse_pong(&reply).is_ok_and(|(got, _)| got == id))
    };
    attempt().unwrap_or(false)
}

/// Whether the boot marks text names `event` (lines `<ms> <event>`; the first mark of an event is the one kept).
pub fn mark_seen(marks: &str, event: &str) -> bool {
    marks.lines().any(|l| l.split_whitespace().nth(1) == Some(event))
}

/// Polls `marks` for `event` until it appears or `wait` passes; true when it was seen. `poll` is the pause between
/// checks.
pub fn wait_for_mark(marks: &Path, event: &str, wait: Duration, poll: Duration) -> bool {
    let start = Instant::now();
    loop {
        if std::fs::read_to_string(marks).is_ok_and(|t| mark_seen(&t, event)) {
            return true;
        }
        if start.elapsed() >= wait {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// `bifrostd desktop-ready`: waits for the first desktop frame (or the ceiling), then starts the after-desktop target
/// without blocking. Returns whether the frame was seen before the ceiling.
pub fn release_after_desktop(marks: &Path) -> bool {
    let seen = wait_for_mark(marks, DESKTOP_READY_MARK, DESKTOP_READY_WAIT, Duration::from_millis(50));
    if !seen {
        eprintln!("bifrostd: no {DESKTOP_READY_MARK} mark within {DESKTOP_READY_WAIT:?}, starting {AFTER_DESKTOP_TARGET} anyway");
    }
    match Command::new("systemctl").args(["start", "--no-block", AFTER_DESKTOP_TARGET]).status() {
        Ok(s) if s.success() => {}
        other => eprintln!("bifrostd: cannot start {AFTER_DESKTOP_TARGET}: {other:?}"),
    }
    seen
}

/// The `pollfd` layout of `poll(2)`: a listening socket is readable when a connection is waiting on it.
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x001;

extern "C" {
    fn chown(path: *const std::ffi::c_char, owner: u32, group: u32) -> i32;
    fn setgroups(size: usize, list: *const u32) -> i32;
    fn setgid(gid: u32) -> i32;
    fn setuid(uid: u32) -> i32;
    fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
}

/// Waits up to `timeout` for one of `fds` to become readable (a connection is waiting). With no fds it just sleeps.
fn wait_readable(fds: &[i32], timeout: Duration) {
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    if fds.is_empty() {
        std::thread::sleep(timeout);
        return;
    }
    let mut set: Vec<PollFd> = fds.iter().map(|&fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
    // SAFETY: `set` is a live array of `set.len()` `pollfd`-layout structs, borrowed exclusively for the whole call.
    unsafe { poll(set.as_mut_ptr(), set.len() as u64, ms) };
}

/// Whether a connection is already waiting on `listener` (a zero-timeout poll).
fn pending(listener: &UnixListener) -> bool {
    let mut p = PollFd { fd: listener.as_raw_fd(), events: POLLIN, revents: 0 };
    // SAFETY: `p` is one live `pollfd`-layout struct for the duration of the call.
    let ready = unsafe { poll(&mut p, 1, 0) };
    ready > 0 && p.revents & POLLIN != 0
}

/// `user = "session"` in a manifest: the user logged in to the desktop.
pub const SESSION_USER: &str = "session";
/// The uid taken for the session user only when no account file names one (logged when it happens).
pub const FALLBACK_UID: u32 = 1000;
/// The mode of every service socket. A service that runs as a user gets the socket chowned to that user, and the
/// sandboxed apps run as the same uid (`bwrap --uid`), so nobody else needs to connect.
pub const SOCKET_MODE: u32 = 0o600;

/// A non-root identity a service is started as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    /// the primary group first, then the supplementary groups from `/etc/group` (so audio-device access is kept)
    pub groups: Vec<u32>,
    /// the session user was not named by the account file, so uid [`FALLBACK_UID`] was taken
    pub fallback: bool,
}

struct PasswdEntry {
    name: String,
    uid: u32,
    gid: u32,
    home: PathBuf,
}

fn passwd_entry(passwd: &str, pick: impl Fn(&PasswdEntry) -> bool) -> Option<PasswdEntry> {
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() != 7 {
                return None;
            }
            Some(PasswdEntry { name: f[0].to_string(), uid: f[2].parse().ok()?, gid: f[3].parse().ok()?, home: PathBuf::from(f[5]) })
        })
        .find(|e| pick(e))
}

/// The primary group, then every other group of `/etc/group` text that lists `name` as a member (never group 0).
fn groups_of(group: &str, name: &str, gid: u32) -> Vec<u32> {
    let mut v = vec![gid];
    for line in group.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() == 4 && f[3].split(',').any(|u| u == name) {
            if let Ok(g) = f[2].parse::<u32>() {
                if g != 0 && !v.contains(&g) {
                    v.push(g);
                }
            }
        }
    }
    v
}

/// The `username=` line of `<root>/account` (what the shell's app launch policy reads), when it is a safe name.
pub fn account_name(text: &str) -> Option<&str> {
    let n = text.lines().find_map(|l| l.strip_prefix("username="))?.trim();
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(n)
}

/// The identity for a manifest `user` value. `Ok(None)` is root (empty or `root`). `session` is the account file's user,
/// else the passwd entry with uid [`FALLBACK_UID`] (`fallback` set); `Err` when no such user is known, so the caller
/// waits instead of ever starting a session service as root. A named user is looked up in passwd; a name that is uid 0
/// is refused.
pub fn resolve_identity(user: &str, account: Option<&str>, passwd: &str, group: &str) -> Result<Option<Identity>, String> {
    let (entry, fallback) = match user {
        "" | "root" => return Ok(None),
        SESSION_USER => match account.and_then(account_name).and_then(|n| passwd_entry(passwd, |e| e.name == n)) {
            Some(e) => (e, false),
            None => (passwd_entry(passwd, |e| e.uid == FALLBACK_UID).ok_or("no desktop session user is known (no account, no uid 1000)")?, true),
        },
        name => (passwd_entry(passwd, |e| e.name == name).ok_or_else(|| format!("unknown user {name}"))?, false),
    };
    if entry.uid == 0 || entry.gid == 0 {
        return Err(format!("{} is root; a service never runs as root by another name", entry.name));
    }
    let groups = groups_of(group, &entry.name, entry.gid);
    Ok(Some(Identity { name: entry.name, uid: entry.uid, gid: entry.gid, home: entry.home, groups, fallback }))
}

/// [`resolve_identity`] for a manifest, reading `<root>/account` and `<etc>/passwd` and `<etc>/group`.
pub fn identity_for(root: &Path, etc: &Path, m: &Manifest) -> Result<Option<Identity>, String> {
    if m.user.is_empty() || m.user == "root" {
        return Ok(None);
    }
    let account = std::fs::read_to_string(root.join("account")).ok();
    let passwd = std::fs::read_to_string(etc.join("passwd")).map_err(|e| format!("cannot read passwd: {e}"))?;
    let group = std::fs::read_to_string(etc.join("group")).unwrap_or_default();
    resolve_identity(&m.user, account.as_deref(), &passwd, &group)
}

/// The uid and gid that own the socket of a service run as `who` (None: root keeps it).
pub fn socket_owner(who: Option<&Identity>) -> Option<(u32, u32)> {
    who.map(|i| (i.uid, i.gid))
}

fn chown_path(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: `c` is a live NUL-terminated path for the duration of the call.
    if unsafe { chown(c.as_ptr(), uid, gid) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Mode [`SOCKET_MODE`] first (so the socket is never reachable by others while it changes hands), then the owner.
fn secure_socket(path: &Path, owner: Option<(u32, u32)>) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE));
    if let Some((uid, gid)) = owner {
        if let Err(e) = chown_path(path, uid, gid) {
            eprintln!("bifrostd: cannot hand {} to {uid}:{gid}: {e}", path.display());
        }
    }
}

/// Binds the service's socket (mode 0600, owned by `who`), replacing a stale file from an earlier run.
fn bind(root: &Path, id: &str, who: Option<&Identity>) -> io::Result<UnixListener> {
    let path = sock_path(root, id);
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    secure_socket(&path, socket_owner(who));
    Ok(l)
}

/// Starts the child with `listener` as its stdin (socket activation). With `who`, the data directory is created for that
/// user and the child drops to it before exec: supplementary groups, then gid, then uid, and no new privileges.
fn spawn(root: &Path, f: &Found, listener: &UnixListener, who: Option<&Identity>) -> io::Result<Child> {
    let data = root.join("appdata").join(&f.manifest.id);
    std::fs::create_dir_all(&data)?;
    if let Some(i) = who {
        chown_path(&data, i.uid, i.gid)?;
    }
    let stdin: OwnedFd = listener.try_clone()?.into();
    let mut cmd = Command::new(f.exec_path());
    cmd.current_dir(&data)
        .env("BIFROST_SERVICE_SOCK", sock_path(root, &f.manifest.id))
        .env("BIFROST_DATA", &data)
        .env("BIFROST_SHELL_DIR", root)
        .env(svc::ACTIVATED_ENV, "1")
        .stdin(Stdio::from(stdin));
    if let Some(i) = who {
        let runtime = PathBuf::from(format!("/run/user/{}", i.uid));
        cmd.env("HOME", &i.home)
            .env("USER", &i.name)
            .env("LOGNAME", &i.name)
            .env("XDG_RUNTIME_DIR", if runtime.is_dir() { runtime } else { data.clone() });
        let (uid, gid, groups) = (i.uid, i.gid, i.groups.clone());
        // SAFETY: the hook runs between fork and exec and only makes syscalls on values it owns (no allocation, no locks).
        unsafe {
            cmd.pre_exec(move || {
                if setgroups(groups.len(), groups.as_ptr()) != 0 || setgid(gid) != 0 || setuid(uid) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // PR_SET_NO_NEW_PRIVS: a decoder bug cannot gain anything through a setuid file
                if prctl(38, 1, 0, 0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn()
}

/// SIGTERM, then SIGKILL after `STOP_GRACE`.
fn stop(child: &mut Child) {
    unsafe { kill(child.id() as i32, 15) };
    let end = Instant::now() + STOP_GRACE;
    while Instant::now() < end {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

struct Running {
    child: Child,
    found: Found,
    started: Instant,
    /// answered a ping since it started
    up: bool,
}

#[derive(Default)]
struct Slot {
    /// the bound socket; a connection waiting here starts the service
    listener: Option<UnixListener>,
    running: Option<Running>,
    backoff: Backoff,
    not_before: Option<Instant>,
    /// a version directory whose swap failed; not retried until `current` moves
    rejected: Option<PathBuf>,
    /// who owns the bound socket now (None: root); changed when the session user changes
    owner: Option<(u32, u32)>,
}

pub struct Supervisor {
    root: PathBuf,
    /// where the OS package keeps its services ([`SYSTEM_SERVICES_DIR`])
    system: PathBuf,
    /// where passwd and group are read (`/etc`)
    etc: PathBuf,
    slots: HashMap<String, Slot>,
    /// one-time log lines ("waits", "fallback") already written
    noted: HashSet<String>,
}

impl Supervisor {
    pub fn new(root: PathBuf) -> Self {
        Self::with_system_dir(root, PathBuf::from(SYSTEM_SERVICES_DIR))
    }

    /// A supervisor that reads the package-provided services from `system` instead of [`SYSTEM_SERVICES_DIR`].
    pub fn with_system_dir(root: PathBuf, system: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(run_dir(&root));
        Supervisor { root, system, etc: PathBuf::from("/etc"), slots: HashMap::new(), noted: HashSet::new() }
    }

    /// Reads passwd and group from `etc` instead of `/etc`.
    pub fn with_etc(mut self, etc: PathBuf) -> Self {
        self.etc = etc;
        self
    }

    /// How many services are running now (started on demand and not yet exited).
    pub fn running(&self) -> usize {
        self.slots.values().filter(|s| s.running.is_some()).count()
    }

    /// Waits up to `max` for a client to wait on an idle service's socket, or for a restart backoff to end. Returns at
    /// once when one does, so a service starts without waiting for the periodic tick.
    pub fn wait(&self, max: Duration) {
        let now = Instant::now();
        let mut timeout = max;
        let mut fds = Vec::new();
        for slot in self.slots.values() {
            let (Some(l), None) = (&slot.listener, &slot.running) else { continue };
            match slot.not_before {
                Some(t) if t > now => timeout = timeout.min(t - now),
                _ => fds.push(l.as_raw_fd()),
            }
        }
        wait_readable(&fds, timeout);
    }

    /// One pass: bind the sockets of new services, reap exits, start idle services that have a waiting connection,
    /// swap changed versions, stop removed services.
    pub fn tick(&mut self) {
        let found = scan_with(&self.root, &self.system);
        let now = Instant::now();
        let gone: Vec<String> = self.slots.keys().filter(|id| !found.iter().any(|f| &f.manifest.id == *id)).cloned().collect();
        for id in gone {
            if let Some(mut slot) = self.slots.remove(&id) {
                eprintln!("bifrostd: {id} removed, stopping");
                if let Some(mut r) = slot.running.take() {
                    stop(&mut r.child);
                }
                let _ = std::fs::remove_file(sock_path(&self.root, &id));
            }
        }
        for f in found {
            let id = f.manifest.id.clone();
            // who the service runs as; a session service whose user is not known yet is neither bound nor started
            // (retried on the next tick), and never falls back to root
            let identity = identity_for(&self.root, &self.etc, &f.manifest);
            match &identity {
                Ok(Some(i)) if i.fallback && self.noted.insert(format!("fallback:{id}")) => {
                    eprintln!("bifrostd: {id}: the account file names no user, running as {} (uid {FALLBACK_UID})", i.name)
                }
                Err(why) if self.noted.insert(format!("wait:{id}")) => eprintln!("bifrostd: {id} waits: {why}"),
                Ok(_) => {
                    self.noted.remove(&format!("wait:{id}"));
                }
                Err(_) => {}
            }
            let known: Option<Option<&Identity>> = identity.as_ref().ok().map(|o| o.as_ref());
            let slot = self.slots.entry(id.clone()).or_default();
            if slot.listener.is_none() {
                let Some(who) = known else { continue };
                match bind(&self.root, &id, who) {
                    Ok(l) => {
                        slot.listener = Some(l);
                        slot.owner = socket_owner(who);
                    }
                    Err(e) => {
                        eprintln!("bifrostd: cannot bind {id}: {e}");
                        continue;
                    }
                }
            } else if let Some(who) = known {
                // the session user changed: the socket follows
                let want = socket_owner(who);
                if slot.owner != want {
                    secure_socket(&sock_path(&self.root, &id), want);
                    slot.owner = want;
                }
            }
            if let Some(r) = &mut slot.running {
                match r.child.try_wait() {
                    Ok(None) => {
                        if !r.up {
                            r.up = healthy(&sock_path(&self.root, &id), &id);
                        }
                    }
                    other => {
                        let ran = r.started.elapsed();
                        eprintln!("bifrostd: {id} exited ({other:?}) after {ran:?}");
                        slot.running = None;
                        slot.not_before = Some(now + slot.backoff.failed(ran));
                    }
                }
            }
            if slot.running.is_none() {
                let due = slot.not_before.is_none_or(|t| now >= t);
                let waiting = slot.listener.as_ref().is_some_and(pending);
                if due && waiting {
                    if let (Some(listener), Some(who)) = (slot.listener.as_ref(), known) {
                        match spawn(&self.root, &f, listener, who) {
                            Ok(child) => {
                                eprintln!("bifrostd: started {id} {} on demand", f.manifest.version);
                                slot.running = Some(Running { child, found: f, started: now, up: false });
                                slot.not_before = None;
                            }
                            Err(e) => {
                                eprintln!("bifrostd: cannot start {id}: {e}");
                                slot.not_before = Some(now + slot.backoff.failed(Duration::ZERO));
                            }
                        }
                    }
                }
            } else if let (true, Some(who)) = (slot.running.as_ref().is_some_and(|r| r.found.dir != f.dir) && slot.rejected.as_ref() != Some(&f.dir), known) {
                let mut old = slot.running.take().expect("checked running above");
                match swap(&self.root, &id, &mut old, &f, who) {
                    Some((new, listener)) => {
                        slot.running = Some(new);
                        slot.listener = Some(listener);
                        slot.owner = socket_owner(who);
                        slot.rejected = None;
                    }
                    None => {
                        slot.running = Some(old);
                        slot.rejected = Some(f.dir.clone());
                    }
                }
            }
        }
    }

    /// Stops every service (the supervisor is leaving).
    pub fn shutdown(&mut self) {
        for (id, mut slot) in self.slots.drain() {
            if let Some(mut r) = slot.running.take() {
                stop(&mut r.child);
            }
            let _ = std::fs::remove_file(sock_path(&self.root, &id));
        }
    }
}

/// Starts the new version on a side socket, waits for it to answer, then retires the old one and moves the side socket
/// into place. None (the old version stays) when the new one never answers. The new version's listener is returned
/// with it, so the supervisor keeps the socket that the running child accepts on.
fn swap(root: &Path, id: &str, old: &mut Running, new: &Found, who: Option<&Identity>) -> Option<(Running, UnixListener)> {
    let side_path = run_dir(root).join(format!("{id}.sock.new"));
    let _ = std::fs::remove_file(&side_path);
    let side = match UnixListener::bind(&side_path) {
        Ok(l) => {
            secure_socket(&side_path, socket_owner(who));
            l
        }
        Err(e) => {
            eprintln!("bifrostd: swap of {id} to {}: cannot bind: {e}", new.manifest.version);
            return None;
        }
    };
    let mut child = match spawn(root, new, &side, who) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bifrostd: swap of {id} to {}: cannot start: {e}", new.manifest.version);
            let _ = std::fs::remove_file(&side_path);
            return None;
        }
    };
    let end = Instant::now() + HEALTH_TIMEOUT;
    let mut ok = false;
    while Instant::now() < end {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if healthy(&side_path, id) {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if !ok {
        eprintln!("bifrostd: swap of {id} to {} failed health check, keeping {}", new.manifest.version, old.found.manifest.version);
        stop(&mut child);
        let _ = std::fs::remove_file(&side_path);
        return None;
    }
    stop(&mut old.child);
    if let Err(e) = std::fs::rename(&side_path, sock_path(root, id)) {
        eprintln!("bifrostd: swap of {id}: cannot move the socket: {e}");
        stop(&mut child);
        return None;
    }
    eprintln!("bifrostd: {id} now {}", new.manifest.version);
    Some((Running { child, found: new.clone(), started: Instant::now(), up: true }, side))
}

#[cfg(test)]
mod tests {
    #[test]
    fn exec_symlink_outside_the_package_is_rejected() {
        let d = std::env::temp_dir().join(format!("bifrostd-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("pkg")).unwrap();
        let dir = std::fs::canonicalize(d.join("pkg")).unwrap();
        std::fs::write(d.join("outside"), b"x").unwrap();
        std::fs::write(dir.join("inside"), b"x").unwrap();
        std::os::unix::fs::symlink(d.join("outside"), dir.join("link")).unwrap();
        assert!(super::exec_inside(&dir, "inside"));
        assert!(!super::exec_inside(&dir, "link"));
        assert!(!super::exec_inside(&dir, "missing"));
        let _ = std::fs::remove_dir_all(&d);
    }

    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn manifest_only_accepts_safe_services() {
        let ok = "id = \"is.olibuijr.audio\"\nversion = \"1.0.0\"\nkind = \"service\"\nexec = \"audiod\"\n";
        assert_eq!(Manifest::parse(ok).unwrap().exec, "audiod");
        assert!(Manifest::parse(&ok.replace("service", "app")).is_none());
        assert!(Manifest::parse(&ok.replace("audiod", "../x")).is_none());
        assert!(Manifest::parse(&ok.replace("is.olibuijr.audio", "a/b")).is_none());
    }

    #[test]
    fn backoff_doubles_to_the_cap_and_resets_after_a_stable_run() {
        let mut b = Backoff::default();
        let secs: Vec<u64> = (0..8).map(|_| b.failed(Duration::from_secs(1)).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(b.failed(STABLE_AFTER).as_secs(), 1);
    }

    #[test]
    fn a_mark_is_matched_by_whole_event_name() {
        let marks = "1200.5 supervisor-start\n3400.0 desktop-ready\n3500.0 desktop-first-ui\n";
        assert!(mark_seen(marks, "desktop-ready"));
        assert!(mark_seen(marks, "supervisor-start"));
        assert!(!mark_seen(marks, "desktop"), "a prefix of an event is not the event");
        assert!(!mark_seen(marks, "desktop-faded"));
        assert!(!mark_seen("", "desktop-ready"));
    }

    #[test]
    fn waiting_for_a_mark_returns_at_once_or_at_the_ceiling() {
        let dir = std::env::temp_dir().join(format!("bifrostd-marks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marks = dir.join("boot-marks");
        std::fs::write(&marks, "10.0 desktop-ready\n").unwrap();
        let t = Instant::now();
        assert!(wait_for_mark(&marks, "desktop-ready", Duration::from_secs(5), Duration::from_millis(10)));
        assert!(t.elapsed() < Duration::from_secs(1));
        // the ceiling: a mark that never comes is reported as not seen once the wait has passed
        let t = Instant::now();
        assert!(!wait_for_mark(&marks, "desktop-faded", Duration::from_millis(60), Duration::from_millis(10)));
        assert!(t.elapsed() >= Duration::from_millis(60));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A service exists on disk but is not started until a client connects to its socket.
    #[test]
    fn a_service_starts_only_when_a_client_is_waiting() {
        let root = std::env::temp_dir().join(format!("bifrostd-activation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let version = root.join("apps/is.test.echo/versions/1.0.0");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join("manifest.toml"), "id = \"is.test.echo\"\nversion = \"1.0.0\"\nkind = \"service\"\nexec = \"svc.sh\"\n").unwrap();
        let exec = version.join("svc.sh");
        std::fs::write(&exec, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&version, root.join("apps/is.test.echo/current")).unwrap();

        let mut sup = Supervisor::with_system_dir(root.clone(), root.join("no-system-services"));
        sup.tick();
        assert!(sock_path(&root, "is.test.echo").exists(), "the socket is bound before anyone connects");
        assert_eq!(sup.running(), 0, "nothing starts without a client");

        let _client = UnixStream::connect(sock_path(&root, "is.test.echo")).unwrap();
        sup.tick();
        assert_eq!(sup.running(), 1, "the waiting connection starts the service");
        sup.shutdown();
        assert_eq!(sup.running(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Writes a service package (`manifest.toml` and an executable) into `dir`.
    fn write_service(dir: &Path, id: &str, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), format!("id = \"{id}\"\nversion = \"{version}\"\nkind = \"service\"\nexec = \"svc.sh\"\n")).unwrap();
        let exec = dir.join("svc.sh");
        std::fs::write(&exec, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrostd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    /// Installs a store copy of `id` the way the store does: `apps/<id>/versions/<v>` with a `current` link.
    fn store_install(root: &Path, id: &str, version: &str) {
        let v = root.join("apps").join(id).join("versions").join(version);
        write_service(&v, id, version);
        let cur = root.join("apps").join(id).join("current");
        let _ = std::fs::remove_file(&cur);
        std::os::unix::fs::symlink(&v, cur).unwrap();
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_newer("0.10.8", "0.9.9"), "10 is more than 9");
        assert!(version_newer("1.0.1", "1.0"));
        assert!(!version_newer("1.0.0", "1.0"), "missing components are 0");
        assert!(!version_newer("0.1.0", "0.1.0"), "equal is not newer");
        assert!(!version_newer("0.1.0", "0.2.0"));
        assert!(version_newer("2.0.0-rc1", "1.9.9"));
    }

    #[test]
    fn package_provided_services_are_found_without_a_store_install() {
        let d = scratch("system-only");
        let (root, system) = (d.join("root"), d.join("system"));
        write_service(&system.join("is.test.audio"), "is.test.audio", "0.1.0");
        let found = scan_with(&root, &system);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.id, "is.test.audio");
        assert_eq!(found[0].dir, system.join("is.test.audio"));
        assert!(found[0].exec_path().is_file());
        assert!(scan(&root).is_empty(), "scan() sees the store only");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_package_directory_must_be_named_after_its_id_and_hold_its_exec() {
        let d = scratch("system-bad");
        let (root, system) = (d.join("root"), d.join("system"));
        write_service(&system.join("wrong-name"), "is.test.audio", "0.1.0");
        std::fs::create_dir_all(system.join("is.test.noexec")).unwrap();
        std::fs::write(system.join("is.test.noexec/manifest.toml"), "id = \"is.test.noexec\"\nversion = \"1\"\nkind = \"service\"\nexec = \"missing\"\n").unwrap();
        assert!(scan_with(&root, &system).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_store_copy_wins_only_when_its_version_is_newer() {
        let d = scratch("precedence");
        let (root, system) = (d.join("root"), d.join("system"));
        write_service(&system.join("is.test.audio"), "is.test.audio", "0.2.0");
        // older store copy (a stale install from before the OS shipped the service): the package wins
        store_install(&root, "is.test.audio", "0.1.0");
        let f = scan_with(&root, &system);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].dir, system.join("is.test.audio"), "an older store copy loses");
        // equal version: the package wins too
        store_install(&root, "is.test.audio", "0.2.0");
        assert_eq!(scan_with(&root, &system)[0].dir, system.join("is.test.audio"), "a tie goes to the package");
        // newer store copy wins
        store_install(&root, "is.test.audio", "0.10.0");
        let f = scan_with(&root, &system);
        assert_eq!(f.len(), 1, "one service per id");
        assert_eq!(f[0].manifest.version, "0.10.0");
        assert!(f[0].dir.starts_with(root.join("apps")));
        // services only in the store are kept alongside, sorted by id
        store_install(&root, "is.test.net", "1.0.0");
        let ids: Vec<String> = scan_with(&root, &system).into_iter().map(|f| f.manifest.id).collect();
        assert_eq!(ids, ["is.test.audio", "is.test.net"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A package-provided service gets its socket under `<root>/run` and starts on demand like a store one.
    #[test]
    fn a_package_provided_service_starts_when_a_client_is_waiting() {
        let d = scratch("system-activation");
        let (root, system) = (d.join("root"), d.join("system"));
        write_service(&system.join("is.olibuijr.audio"), "is.olibuijr.audio", "0.1.0");
        let mut sup = Supervisor::with_system_dir(root.clone(), system);
        sup.tick();
        let sock = sock_path(&root, "is.olibuijr.audio");
        assert!(sock.exists(), "the socket stays under <root>/run so clients and the app sandbox find it");
        assert_eq!(sup.running(), 0);
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, SOCKET_MODE, "a service socket is private to its owner; apps run as the same uid");
        let _client = UnixStream::connect(&sock).unwrap();
        sup.tick();
        assert_eq!(sup.running(), 1);
        sup.shutdown();
        let _ = std::fs::remove_dir_all(&d);
    }

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\naudio:x:975:975:audio:/:/usr/bin/nologin\nolibuijr:x:1000:1000::/home/olibuijr:/bin/bash\nguest:x:1001:1001::/home/guest:/bin/bash\n";
    const GROUP: &str = "root:x:0:root\naudio:x:975:olibuijr\nwheel:x:998:olibuijr,guest\nolibuijr:x:1000:\nguest:x:1001:\nvideo:x:12:root,olibuijr\n";

    #[test]
    fn the_manifest_names_who_the_service_runs_as() {
        let m = |user: &str| format!("id = \"is.olibuijr.audio\"\nversion = \"1\"\nkind = \"service\"\nexec = \"audiod\"\n{user}");
        assert_eq!(Manifest::parse(&m("")).unwrap().user, "", "root by default");
        assert_eq!(Manifest::parse(&m("user = \"session\"\n")).unwrap().user, "session");
        assert_eq!(Manifest::parse(&m("user = \"audio\"\n")).unwrap().user, "audio");
        assert!(Manifest::parse(&m("user = \"a/b\"\n")).is_none(), "an unsafe user name is refused");
        assert!(Manifest::parse(&m("user = \"x y\"\n")).is_none());
    }

    #[test]
    fn the_shipped_manifests_run_audio_as_the_session_and_the_rest_as_root() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../system");
        let user = |d: &str| Manifest::parse(&std::fs::read_to_string(dir.join(d).join("manifest.toml")).unwrap()).unwrap().user;
        assert_eq!(user("audio"), "session");
        assert_eq!(user("net"), "");
        assert_eq!(user("update"), "");
    }

    #[test]
    fn root_services_have_no_identity_and_the_socket_stays_with_root() {
        assert_eq!(resolve_identity("", None, PASSWD, GROUP), Ok(None));
        assert_eq!(resolve_identity("root", None, PASSWD, GROUP), Ok(None));
        assert_eq!(socket_owner(None), None);
        assert_eq!(SOCKET_MODE, 0o600);
    }

    #[test]
    fn the_session_user_comes_from_the_account_file_with_its_groups() {
        let who = resolve_identity("session", Some("username=guest\n"), PASSWD, GROUP).unwrap().unwrap();
        assert_eq!((who.name.as_str(), who.uid, who.gid), ("guest", 1001, 1001));
        assert_eq!(who.home, PathBuf::from("/home/guest"));
        assert_eq!(who.groups, [1001, 998], "primary group, then the supplementary ones");
        assert!(!who.fallback);
        assert_eq!(socket_owner(Some(&who)), Some((1001, 1001)), "the socket goes to the session user");
        let me = resolve_identity("session", Some("username=olibuijr\n"), PASSWD, GROUP).unwrap().unwrap();
        assert_eq!(me.groups, [1000, 975, 998, 12], "audio-device access (group audio) is kept");
    }

    #[test]
    fn a_session_service_falls_back_to_uid_1000_only_when_the_account_names_nobody_and_says_so() {
        let who = resolve_identity("session", None, PASSWD, GROUP).unwrap().unwrap();
        assert_eq!((who.uid, who.fallback), (1000, true));
        let unknown = resolve_identity("session", Some("username=ghost\n"), PASSWD, GROUP).unwrap().unwrap();
        assert_eq!((unknown.uid, unknown.fallback), (1000, true));
    }

    #[test]
    fn a_session_service_is_refused_without_a_session_user_and_never_runs_as_root() {
        let only_system = "root:x:0:0:root:/root:/bin/bash\naudio:x:975:975:audio:/:/usr/bin/nologin\n";
        assert!(resolve_identity("session", None, only_system, GROUP).is_err());
        assert!(resolve_identity("session", Some("username=root\n"), only_system, GROUP).is_err(), "root is no session user");
        assert!(resolve_identity("root-by-name", None, PASSWD, GROUP).is_err(), "an unknown name is refused");
        assert!(resolve_identity("session", Some("username=root\n"), PASSWD, GROUP).is_err(), "an account file naming root is refused, not replaced by someone else");
        // a name that resolves to uid 0 is refused outright
        let alias = "toor:x:0:0::/root:/bin/sh\n";
        assert!(resolve_identity("toor", None, alias, "").is_err());
    }

    #[test]
    fn the_account_file_name_must_be_safe() {
        assert_eq!(account_name("username=olibuijr\nother=1\n"), Some("olibuijr"));
        assert_eq!(account_name("username=\n"), None);
        assert_eq!(account_name("username=a/b\n"), None);
        assert_eq!(account_name("nothing\n"), None);
    }

    /// A session service is neither bound nor started while no session user is known.
    #[test]
    fn a_session_service_waits_for_a_session_user() {
        let d = scratch("session-wait");
        let (root, system, etc) = (d.join("root"), d.join("system"), d.join("etc"));
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();
        std::fs::write(etc.join("group"), "root:x:0:\n").unwrap();
        write_service(&system.join("is.olibuijr.audio"), "is.olibuijr.audio", "0.1.0");
        std::fs::write(
            system.join("is.olibuijr.audio/manifest.toml"),
            "id = \"is.olibuijr.audio\"\nversion = \"0.1.0\"\nkind = \"service\"\nexec = \"svc.sh\"\nuser = \"session\"\n",
        )
        .unwrap();
        let mut sup = Supervisor::with_system_dir(root.clone(), system).with_etc(etc);
        sup.tick();
        assert!(!sock_path(&root, "is.olibuijr.audio").exists(), "no socket: nothing may bind or start it as root");
        assert_eq!(sup.running(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_socket_is_named_after_its_service_under_run() {
        assert_eq!(sock_path(Path::new("/var/lib/springboard"), "is.olibuijr.audio"), PathBuf::from("/var/lib/springboard/run/is.olibuijr.audio.sock"));
        assert_eq!(run_dir(Path::new("/x")), PathBuf::from("/x/run"));
    }

    #[test]
    fn marks_skip_blank_and_one_word_lines() {
        let marks = "\n desktop-ready\nbroken\n12.0 desktop-ready\n";
        assert!(mark_seen(marks, "desktop-ready"), "a line with a time and an event name counts");
        assert!(!mark_seen("\n\n", "desktop-ready"));
        assert!(!mark_seen("broken\n", "broken"), "a line with no time has no event");
    }

    #[test]
    fn the_desktop_ready_names_and_wait_are_the_contract() {
        assert_eq!(DESKTOP_READY_MARK, "desktop-ready");
        assert_eq!(AFTER_DESKTOP_TARGET, "bifrost-after-desktop.target");
        assert!(DESKTOP_READY_WAIT > Duration::from_secs(4), "the reveal's own ceiling is 4 s");
        assert!(TICK <= DESKTOP_READY_WAIT);
    }
}
