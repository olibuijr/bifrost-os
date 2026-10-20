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
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
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
                _ => {}
            }
        }
        let safe_id = !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        let safe_exec = !m.exec.is_empty() && !m.exec.starts_with('/') && !m.exec.split('/').any(|p| p == "..");
        // only `service` packages: the shell's kind table (src/shell/kinds.rs) is the reference
        (m.kind == "service" && safe_id && safe_exec).then_some(m)
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

/// Services whose socket any local account may connect to: the apps run under their own accounts (inside the
/// sandbox, which binds only this socket) and bifrostd runs as root, so the default 0755 would refuse them.
const OPEN_SOCKETS: &[&str] = &["is.olibuijr.audio"];

/// The socket file mode for `id`: world-connectable for [`OPEN_SOCKETS`], else the umask's default (root only).
pub fn socket_mode(id: &str) -> Option<u32> {
    OPEN_SOCKETS.contains(&id).then_some(0o666)
}

/// Applies [`socket_mode`] to the bound socket file at `path`.
fn set_socket_mode(path: &Path, id: &str) {
    if let Some(mode) = socket_mode(id) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
}

/// Binds the service's socket, replacing a stale file from an earlier run.
fn bind(root: &Path, id: &str) -> io::Result<UnixListener> {
    let path = sock_path(root, id);
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    set_socket_mode(&path, id);
    Ok(l)
}

/// Starts the child with `listener` as its stdin (socket activation).
fn spawn(root: &Path, f: &Found, listener: &UnixListener) -> io::Result<Child> {
    let data = root.join("appdata").join(&f.manifest.id);
    std::fs::create_dir_all(&data)?;
    let stdin: OwnedFd = listener.try_clone()?.into();
    Command::new(f.exec_path())
        .current_dir(&data)
        .env("BIFROST_SERVICE_SOCK", sock_path(root, &f.manifest.id))
        .env("BIFROST_DATA", &data)
        .env("BIFROST_SHELL_DIR", root)
        .env(svc::ACTIVATED_ENV, "1")
        .stdin(Stdio::from(stdin))
        .spawn()
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
}

pub struct Supervisor {
    root: PathBuf,
    /// where the OS package keeps its services ([`SYSTEM_SERVICES_DIR`])
    system: PathBuf,
    slots: HashMap<String, Slot>,
}

impl Supervisor {
    pub fn new(root: PathBuf) -> Self {
        Self::with_system_dir(root, PathBuf::from(SYSTEM_SERVICES_DIR))
    }

    /// A supervisor that reads the package-provided services from `system` instead of [`SYSTEM_SERVICES_DIR`].
    pub fn with_system_dir(root: PathBuf, system: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(run_dir(&root));
        Supervisor { root, system, slots: HashMap::new() }
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
            let slot = self.slots.entry(id.clone()).or_default();
            if slot.listener.is_none() {
                match bind(&self.root, &id) {
                    Ok(l) => slot.listener = Some(l),
                    Err(e) => {
                        eprintln!("bifrostd: cannot bind {id}: {e}");
                        continue;
                    }
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
                    if let Some(listener) = slot.listener.as_ref() {
                        match spawn(&self.root, &f, listener) {
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
            } else if slot.running.as_ref().is_some_and(|r| r.found.dir != f.dir) && slot.rejected.as_ref() != Some(&f.dir) {
                let mut old = slot.running.take().expect("checked running above");
                match swap(&self.root, &id, &mut old, &f) {
                    Some((new, listener)) => {
                        slot.running = Some(new);
                        slot.listener = Some(listener);
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
fn swap(root: &Path, id: &str, old: &mut Running, new: &Found) -> Option<(Running, UnixListener)> {
    let side_path = run_dir(root).join(format!("{id}.sock.new"));
    let _ = std::fs::remove_file(&side_path);
    let side = match UnixListener::bind(&side_path) {
        Ok(l) => {
            set_socket_mode(&side_path, id);
            l
        }
        Err(e) => {
            eprintln!("bifrostd: swap of {id} to {}: cannot bind: {e}", new.manifest.version);
            return None;
        }
    };
    let mut child = match spawn(root, new, &side) {
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
        assert_eq!(mode, 0o666, "apps run under their own accounts and must reach the audio socket");
        let _client = UnixStream::connect(&sock).unwrap();
        sup.tick();
        assert_eq!(sup.running(), 1);
        sup.shutdown();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_the_audio_socket_is_open_to_every_account() {
        assert_eq!(socket_mode("is.olibuijr.audio"), Some(0o666));
        assert_eq!(socket_mode("is.olibuijr.net"), None);
        assert_eq!(socket_mode("is.olibuijr.update"), None);
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
