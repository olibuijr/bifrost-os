//! `bifrostd`: keeps the `kind = "service"` packages running (docs/services.md).
//!
//! Services live like apps, under `<root>/apps/<id>/current/` with a `manifest.toml`. Each runs as
//! a child process that listens on the unix socket named by `BIFROST_SERVICE_SOCK` and answers
//! the `Ping` of `bifrost_proto::svc::common`. A service that exits is restarted with backoff and
//! never given up on; a new `current` is swapped in only after it answers a ping, else the old
//! version keeps running and the new one is not retried until `current` changes again.

use bifrost_proto::svc::{self, common};
use std::collections::HashMap;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A run this long counts as healthy and resets the backoff.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
pub const STOP_GRACE: Duration = Duration::from_secs(3);

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

pub fn scan(root: &Path) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(root.join("apps")) else { return Vec::new() };
    let mut v: Vec<Found> = rd
        .flatten()
        .filter_map(|e| {
            let cur = e.path().join("current");
            let dir = std::fs::canonicalize(&cur).ok()?;
            let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("manifest.toml")).ok()?)?;
            (manifest.id == e.file_name().to_string_lossy() && dir.join(&manifest.exec).is_file()).then_some(Found { manifest, dir })
        })
        .collect();
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

fn spawn(root: &Path, f: &Found, sock: &Path) -> io::Result<Child> {
    let data = root.join("appdata").join(&f.manifest.id);
    std::fs::create_dir_all(&data)?;
    let _ = std::fs::remove_file(sock);
    Command::new(f.exec_path())
        .current_dir(&data)
        .env("BIFROST_SERVICE_SOCK", sock)
        .env("BIFROST_DATA", &data)
        .env("BIFROST_SHELL_DIR", root)
        .stdin(Stdio::null())
        .spawn()
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
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
    running: Option<Running>,
    backoff: Backoff,
    not_before: Option<Instant>,
    /// a version directory whose swap failed; not retried until `current` moves
    rejected: Option<PathBuf>,
}

pub struct Supervisor {
    root: PathBuf,
    slots: HashMap<String, Slot>,
}

impl Supervisor {
    pub fn new(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(run_dir(&root));
        Supervisor { root, slots: HashMap::new() }
    }

    /// One pass: reap exits, start what is missing or due, swap changed versions, stop removed services.
    pub fn tick(&mut self) {
        let found = scan(&self.root);
        let now = Instant::now();
        let gone: Vec<String> = self.slots.keys().filter(|id| !found.iter().any(|f| &f.manifest.id == *id)).cloned().collect();
        for id in gone {
            if let Some(Slot { running: Some(mut r), .. }) = self.slots.remove(&id) {
                eprintln!("bifrostd: {id} removed, stopping");
                stop(&mut r.child);
                let _ = std::fs::remove_file(sock_path(&self.root, &id));
            }
        }
        for f in found {
            let id = f.manifest.id.clone();
            let slot = self.slots.entry(id.clone()).or_default();
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
                if slot.not_before.is_none_or(|t| now >= t) {
                    let sock = sock_path(&self.root, &id);
                    match spawn(&self.root, &f, &sock) {
                        Ok(child) => {
                            eprintln!("bifrostd: started {id} {}", f.manifest.version);
                            slot.running = Some(Running { child, found: f, started: now, up: false });
                        }
                        Err(e) => {
                            eprintln!("bifrostd: cannot start {id}: {e}");
                            slot.not_before = Some(now + slot.backoff.failed(Duration::ZERO));
                        }
                    }
                }
            } else if slot.running.as_ref().is_some_and(|r| r.found.dir != f.dir) && slot.rejected.as_ref() != Some(&f.dir) {
                let mut old = slot.running.take().unwrap();
                match swap(&self.root, &mut old, &f) {
                    Some(new) => {
                        slot.running = Some(new);
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
        for (id, slot) in self.slots.drain() {
            if let Some(mut r) = slot.running {
                stop(&mut r.child);
                let _ = std::fs::remove_file(sock_path(&self.root, &id));
            }
        }
    }
}

/// Starts the new version on a side socket, waits for it to answer, then retires the old one and
/// moves the socket into place. None (the old version stays) when the new one never answers.
fn swap(root: &Path, old: &mut Running, new: &Found) -> Option<Running> {
    let id = &new.manifest.id;
    let side = run_dir(root).join(format!("{id}.sock.new"));
    let mut child = match spawn(root, new, &side) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bifrostd: swap of {id} to {}: cannot start: {e}", new.manifest.version);
            return None;
        }
    };
    let end = Instant::now() + HEALTH_TIMEOUT;
    let mut ok = false;
    while Instant::now() < end {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if healthy(&side, id) {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if !ok {
        eprintln!("bifrostd: swap of {id} to {} failed health check, keeping {}", new.manifest.version, old.found.manifest.version);
        stop(&mut child);
        let _ = std::fs::remove_file(&side);
        return None;
    }
    stop(&mut old.child);
    let main = sock_path(root, id);
    if let Err(e) = std::fs::rename(&side, &main) {
        eprintln!("bifrostd: swap of {id}: cannot move the socket: {e}");
        stop(&mut child);
        return None;
    }
    eprintln!("bifrostd: {id} now {}", new.manifest.version);
    Some(Running { child, found: new.clone(), started: Instant::now(), up: true })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
