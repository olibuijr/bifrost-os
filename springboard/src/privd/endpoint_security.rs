//! Endpoint security, the root half: owns the fanotify descriptor, feeds `bifrost_core::endpoint_security` from it, and
//! hands the matching events to the apps that subscribed.
//!
//! Who may use it: the methods are served only to the shell (`server::dispatch` refuses every other peer), and the shell
//! passes an app's subscription on only when that app declares the `endpoint-security` entitlement in its manifest
//! (`shell/services/endpoint_security.rs`). An app never reaches this socket itself.
//!
//! Methods (params are a JSON object; `app` is the id the shell vouches for):
//! - `endpoint_security.subscribe {app, kinds, auth?, muted?}` -> `{id}`: starts the feed on the first subscription and
//!   marks the file systems for the union of what the subscribers want;
//! - `endpoint_security.poll {app, id, max?}` -> `{events: [..], dropped}`: the queued events, oldest first;
//! - `endpoint_security.answer {app, id, round, allow}` -> `{resolved}`: an app's answer to an authorization event;
//! - `endpoint_security.unsubscribe {app, id}` -> `{}`: any authorization the app still owed is allowed.
//!
//! What it never does: read a file's contents (only the path, from `/proc/self/fd`, and the process id leave), keep an
//! event for a subscriber that is gone, or hold the system up: an authorization nobody answers within
//! [`AUTH_DEADLINE_SECS`] is allowed, and the events of this process itself are always allowed.
//!
//! [`Hub`] is the whole of the logic and holds no descriptor, so the tests drive it with made-up events; [`Shared`] is
//! the thin part that talks to the kernel.
use super::{obj, PErr};
use bifrost_core::endpoint_security::{self as es, AuthRound, Decision, Delivered, RawEvent, Router, Subscription};
use bifrost_core::json::Json;
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const M_SUBSCRIBE: &str = "endpoint_security.subscribe";
pub const M_UNSUBSCRIBE: &str = "endpoint_security.unsubscribe";
pub const M_POLL: &str = "endpoint_security.poll";
pub const M_ANSWER: &str = "endpoint_security.answer";

/// Events kept for one subscriber that is not polling; the oldest go first and are counted.
pub const QUEUE_MAX: usize = 1024;
/// Subscriptions in all, and for one app.
pub const MAX_SUBSCRIPTIONS: usize = 16;
pub const MAX_PER_APP: usize = 2;
/// How long an authorization waits for its answers before it is allowed.
pub const AUTH_DEADLINE_SECS: f64 = 2.0;
/// Events one poll returns at most.
pub const POLL_MAX: usize = 256;

/// The methods this module serves.
pub fn handles(method: &str) -> bool {
    matches!(method, M_SUBSCRIBE | M_UNSUBSCRIBE | M_POLL | M_ANSWER)
}

/// An authorization event that has its answer: the kernel gets `reply` on its descriptor, then `fd` is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub fd: i32,
    pub reply: [u8; 8],
    pub timed_out: bool,
}

/// What the kernel's event needs next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    /// delivered to whoever wanted it; close the event's descriptor
    Notified,
    /// nobody has to be asked: write this reply and close the descriptor
    Answer([u8; 8]),
    /// subscribers were asked: keep the descriptor open until the round resolves
    Waiting,
}

struct Pending {
    round: AuthRound,
    deadline: f64,
}

/// The subscriptions, their queues and the authorizations in flight.
#[derive(Default)]
pub struct Hub {
    router: Router,
    owners: BTreeMap<u32, String>,
    queues: BTreeMap<u32, VecDeque<Delivered>>,
    rounds: BTreeMap<u64, Pending>,
    next_round: u64,
    /// events dropped from a full queue, since the start
    pub dropped: u64,
}

/// An app id as the shell knows it: short, with only the characters a package id has.
pub fn valid_app(app: &str) -> bool {
    !app.is_empty() && app.len() <= 128 && app.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// The kernel mask that covers every subscription.
    pub fn mask(&self) -> u64 {
        self.router.mask()
    }

    pub fn subscriptions(&self) -> usize {
        self.owners.len()
    }

    fn owned(&self, app: &str, id: u32) -> Result<(), String> {
        if self.owners.get(&id).is_some_and(|o| o == app) {
            Ok(())
        } else {
            Err("no such subscription".into())
        }
    }

    pub fn subscribe(&mut self, app: &str, sub: Subscription) -> Result<u32, String> {
        if !valid_app(app) {
            return Err("invalid app id".into());
        }
        if self.owners.len() >= MAX_SUBSCRIPTIONS || self.owners.values().filter(|o| *o == app).count() >= MAX_PER_APP {
            return Err("too many subscriptions".into());
        }
        let id = self.router.subscribe(sub);
        self.owners.insert(id, app.to_string());
        self.queues.insert(id, VecDeque::new());
        Ok(id)
    }

    /// Ends a subscription. The authorizations it still owed are allowed for it, so none waits on a subscriber that is
    /// gone; those that complete are returned for the caller to write.
    pub fn unsubscribe(&mut self, app: &str, id: u32) -> Result<Vec<Resolved>, String> {
        self.owned(app, id)?;
        self.router.unsubscribe(id);
        self.owners.remove(&id);
        self.queues.remove(&id);
        for p in self.rounds.values_mut() {
            p.round.answer(id, Decision::Allow);
        }
        let done: Vec<u64> = self.rounds.iter().filter(|(_, p)| p.round.outcome(false).is_some()).map(|(r, _)| *r).collect();
        Ok(done
            .into_iter()
            .filter_map(|r| {
                let p = self.rounds.remove(&r)?;
                let o = p.round.outcome(false)?;
                Some(Resolved { fd: p.round.fd, reply: p.round.reply(o), timed_out: false })
            })
            .collect())
    }

    /// Every subscription of `app` (an app that exited).
    pub fn unsubscribe_app(&mut self, app: &str) -> Vec<Resolved> {
        let ids: Vec<u32> = self.owners.iter().filter(|(_, o)| *o == app).map(|(i, _)| *i).collect();
        ids.into_iter().flat_map(|id| self.unsubscribe(app, id).unwrap_or_default()).collect()
    }

    fn push(&mut self, id: u32, d: Delivered) {
        let q = self.queues.entry(id).or_default();
        if q.len() >= QUEUE_MAX {
            q.pop_front();
            self.dropped += 1;
        }
        q.push_back(d);
    }

    /// Takes one kernel event with its resolved path: queues it for the subscribers that want it and, for an
    /// authorization, opens a round that ends `AUTH_DEADLINE_SECS` after `now`.
    pub fn dispatch(&mut self, ev: &RawEvent, path: &str, now: f64) -> Dispatch {
        let kinds = ev.kinds();
        for id in self.router.notify_targets(ev, path) {
            self.push(id, Delivered { kinds: kinds.clone(), auth: None, pid: ev.pid, path: path.to_string(), round: None });
        }
        let Some(round) = self.router.auth_round(ev, path) else { return Dispatch::Notified };
        if let Some(o) = round.outcome(false) {
            return Dispatch::Answer(round.reply(o));
        }
        self.next_round += 1;
        let rid = self.next_round;
        for id in round.asked().to_vec() {
            self.push(id, Delivered { kinds: Vec::new(), auth: ev.auth(), pid: ev.pid, path: path.to_string(), round: Some(rid) });
        }
        self.rounds.insert(rid, Pending { round, deadline: now + AUTH_DEADLINE_SECS });
        Dispatch::Waiting
    }

    /// The queued events of one subscription, oldest first, at most `max`.
    pub fn poll(&mut self, app: &str, id: u32, max: usize) -> Result<Vec<Delivered>, String> {
        self.owned(app, id)?;
        let q = self.queues.entry(id).or_default();
        let n = max.min(POLL_MAX).min(q.len());
        Ok(q.drain(..n).collect())
    }

    /// A subscriber's answer to an authorization round. Returns the resolution when this answer completes the round.
    pub fn answer(&mut self, app: &str, id: u32, round: u64, decision: Decision) -> Result<Option<Resolved>, String> {
        self.owned(app, id)?;
        let outcome = {
            let p = self.rounds.get_mut(&round).ok_or("no such authorization")?;
            p.round.answer(id, decision);
            p.round.outcome(false)
        };
        match outcome {
            Some(o) => {
                let p = self.rounds.remove(&round).ok_or("no such authorization")?;
                Ok(Some(Resolved { fd: p.round.fd, reply: p.round.reply(o), timed_out: false }))
            }
            None => Ok(None),
        }
    }

    /// The rounds whose deadline has passed at `now`, ended with the answers that arrived: allowed, as a stuck security
    /// app must not freeze the computer.
    pub fn expire(&mut self, now: f64) -> Vec<Resolved> {
        let due: Vec<u64> = self.rounds.iter().filter(|(_, p)| now >= p.deadline).map(|(r, _)| *r).collect();
        due.into_iter()
            .filter_map(|r| {
                let p = self.rounds.remove(&r)?;
                let o = p.round.outcome(true)?;
                Some(Resolved { fd: p.round.fd, reply: p.round.reply(o), timed_out: o.timed_out })
            })
            .collect()
    }
}

/// The mount points of the real file systems in a `/proc/self/mountinfo` text, one per device: fanotify marks the whole
/// file system, so a second mount of the same device (a btrfs subvolume) adds nothing. Pseudo file systems are skipped.
pub fn fs_mounts(mountinfo: &str) -> Vec<String> {
    const REAL: [&str; 9] = ["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs3"];
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let Some(point) = left.split_whitespace().nth(4) else { continue };
        let mut r = right.split_whitespace();
        let (Some(fstype), Some(source)) = (r.next(), r.next()) else { continue };
        // an octal escape (a space in the name) is skipped rather than guessed at
        if !REAL.contains(&fstype) || point.contains('\\') || seen.iter().any(|s| s == source) {
            continue;
        }
        seen.push(source.to_string());
        out.push(point.to_string());
    }
    out
}

// fanotify(7) constants and calls; the libc wrappers are in glibc, which the daemon already links.
const FAN_CLOEXEC: u32 = 0x1;
const FAN_NONBLOCK: u32 = 0x2;
const FAN_CLASS_CONTENT: u32 = 0x4;
const FAN_MARK_ADD: u32 = 0x1;
const FAN_MARK_FLUSH: u32 = 0x80;
const FAN_MARK_FILESYSTEM: u32 = 0x100;
const O_CLOEXEC: u32 = 0o2_000_000;
const AT_FDCWD: i32 = -100;
const POLLIN: i16 = 0x1;

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn fanotify_init(flags: u32, event_f_flags: u32) -> i32;
    fn fanotify_mark(fanotify_fd: i32, flags: u32, mask: u64, dirfd: i32, pathname: *const std::os::raw::c_char) -> i32;
    fn poll(fds: *mut PollFd, nfds: std::os::raw::c_ulong, timeout: i32) -> i32;
}

/// Closes an event's descriptor (the kernel gave this process one per event).
fn close_fd(fd: i32) {
    if fd >= 0 {
        // SAFETY: the fd came from the kernel for this event and is closed exactly once, here.
        drop(unsafe { File::from_raw_fd(fd) });
    }
}

/// The path an event's descriptor names, without a " (deleted)" suffix. Only the path is read, never the contents.
fn path_of(fd: i32) -> String {
    let p = std::fs::read_link(format!("/proc/self/fd/{fd}")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    p.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(p)
}

/// The kernel side: the fanotify descriptor, the marked mounts and the hub behind one lock.
pub struct Shared {
    hub: Mutex<Hub>,
    fan: File,
    marks: Vec<String>,
    started: Instant,
}

impl Shared {
    fn open() -> Result<Shared, String> {
        // SAFETY: plain syscall wrapper; the result is checked.
        let fd = unsafe { fanotify_init(FAN_CLOEXEC | FAN_NONBLOCK | FAN_CLASS_CONTENT, O_CLOEXEC) };
        if fd < 0 {
            return Err(format!("fanotify_init: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: `fd` is a fresh descriptor this process owns.
        let fan = unsafe { File::from_raw_fd(fd) };
        let marks = fs_mounts(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default());
        if marks.is_empty() {
            return Err("no file system to watch".into());
        }
        Ok(Shared { hub: Mutex::new(Hub::new()), fan, marks, started: Instant::now() })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Hub> {
        self.hub.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Marks every watched file system for `mask`, replacing the earlier marks (0 removes them all).
    fn remark(&self, mask: u64) -> Result<(), String> {
        let fd = self.fan.as_raw_fd();
        // SAFETY: no path is read for a flush; the call only touches this group's marks.
        unsafe { fanotify_mark(fd, FAN_MARK_FLUSH | FAN_MARK_FILESYSTEM, 0, AT_FDCWD, std::ptr::null()) };
        if mask == 0 {
            return Ok(());
        }
        let mut marked = 0;
        for m in &self.marks {
            let Ok(c) = std::ffi::CString::new(m.as_str()) else { continue };
            // SAFETY: `c` is a NUL-terminated path that lives through the call.
            if unsafe { fanotify_mark(fd, FAN_MARK_ADD | FAN_MARK_FILESYSTEM, mask, AT_FDCWD, c.as_ptr()) } == 0 {
                marked += 1;
            }
        }
        if marked == 0 {
            return Err(format!("fanotify_mark: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Writes the kernel's reply for a resolved authorization and closes its descriptor.
    fn resolve(&self, r: Resolved) {
        let _ = (&self.fan).write_all(&r.reply);
        close_fd(r.fd);
    }

    fn handle(&self, ev: &RawEvent) {
        if ev.overflowed() || ev.fd < 0 {
            return;
        }
        // this process's own opens are never reported or held: it would wait on itself
        if ev.pid == std::process::id() as i32 {
            if ev.auth().is_some() {
                let _ = (&self.fan).write_all(&es::response_bytes(ev.fd, Decision::Allow));
            }
            close_fd(ev.fd);
            return;
        }
        let path = path_of(ev.fd);
        let d = self.lock().dispatch(ev, &path, self.now());
        match d {
            Dispatch::Notified => close_fd(ev.fd),
            Dispatch::Answer(reply) => self.resolve(Resolved { fd: ev.fd, reply, timed_out: false }),
            Dispatch::Waiting => {}
        }
    }

    /// The feed: wait for events (waking every 100 ms to let a stuck authorization through), read, route.
    fn run(&self) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut p = PollFd { fd: self.fan.as_raw_fd(), events: POLLIN, revents: 0 };
            // SAFETY: `p` is one valid pollfd for the call.
            let n = unsafe { poll(&mut p, 1, 100) };
            let due = self.lock().expire(self.now());
            for r in due {
                self.resolve(r);
            }
            if n <= 0 {
                continue;
            }
            let got = match (&self.fan).read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => n,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => continue,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };
            if let Ok(events) = es::parse_events(&buf[..got]) {
                for ev in &events {
                    self.handle(ev);
                }
            }
        }
    }
}

/// The running feed, started by the first subscription.
fn shared() -> Result<Arc<Shared>, PErr> {
    static FEED: OnceLock<Mutex<Option<Arc<Shared>>>> = OnceLock::new();
    let cell = FEED.get_or_init(|| Mutex::new(None));
    let mut slot = cell.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = slot.as_ref() {
        return Ok(Arc::clone(s));
    }
    let s = Arc::new(Shared::open().map_err(PErr::internal)?);
    let runner = Arc::clone(&s);
    std::thread::Builder::new().name("endpoint-security".into()).spawn(move || runner.run()).map_err(|e| PErr::internal(e.to_string()))?;
    *slot = Some(Arc::clone(&s));
    Ok(s)
}

fn app_of(p: &Json) -> Result<&str, PErr> {
    let app = p.get("app").and_then(Json::as_str).ok_or_else(|| PErr::bad("missing app"))?;
    if valid_app(app) {
        Ok(app)
    } else {
        Err(PErr::bad("invalid app id"))
    }
}

fn id_of(p: &Json) -> Result<u32, PErr> {
    p.get("id").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| PErr::bad("missing id"))
}

/// Checks the parameters of a subscribe request, before any descriptor exists.
pub fn parse_subscribe(p: &Json) -> Result<(String, Subscription), PErr> {
    let app = app_of(p)?.to_string();
    let sub = Subscription::from_json(p).map_err(PErr::bad)?;
    Ok((app, sub))
}

pub fn call(method: &str, p: &Json) -> Result<Json, PErr> {
    match method {
        M_SUBSCRIBE => {
            let (app, sub) = parse_subscribe(p)?;
            let feed = shared()?;
            let (id, mask) = {
                let mut hub = feed.lock();
                let id = hub.subscribe(&app, sub).map_err(PErr::bad)?;
                (id, hub.mask())
            };
            if let Err(e) = feed.remark(mask) {
                let resolved = feed.lock().unsubscribe(&app, id).unwrap_or_default();
                resolved.into_iter().for_each(|r| feed.resolve(r));
                return Err(PErr::internal(e));
            }
            Ok(obj(vec![("id", Json::Num(f64::from(id)))]))
        }
        M_UNSUBSCRIBE => {
            let (app, id) = (app_of(p)?, id_of(p)?);
            let feed = shared()?;
            let (resolved, mask) = {
                let mut hub = feed.lock();
                let resolved = hub.unsubscribe(app, id).map_err(PErr::bad)?;
                (resolved, hub.mask())
            };
            resolved.into_iter().for_each(|r| feed.resolve(r));
            let _ = feed.remark(mask);
            Ok(obj(vec![]))
        }
        M_POLL => {
            let (app, id) = (app_of(p)?, id_of(p)?);
            let max = p.get("max").and_then(Json::as_u64).map_or(POLL_MAX, |m| m as usize);
            let feed = shared()?;
            let mut hub = feed.lock();
            let events = hub.poll(app, id, max).map_err(PErr::bad)?;
            Ok(obj(vec![("events", Json::Arr(events.iter().map(Delivered::to_json).collect())), ("dropped", Json::Num(hub.dropped as f64))]))
        }
        M_ANSWER => {
            let (app, id) = (app_of(p)?, id_of(p)?);
            let round = p.get("round").and_then(Json::as_u64).ok_or_else(|| PErr::bad("missing round"))?;
            let allow = match p.get("allow") {
                Some(Json::Bool(b)) => *b,
                _ => return Err(PErr::bad("allow must be a boolean")),
            };
            let feed = shared()?;
            let resolved = feed.lock().answer(app, id, round, if allow { Decision::Allow } else { Decision::Deny }).map_err(PErr::bad)?;
            let done = resolved.is_some();
            if let Some(r) = resolved {
                feed.resolve(r);
            }
            Ok(obj(vec![("resolved", Json::Bool(done))]))
        }
        _ => Err(PErr::bad(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::endpoint_security::{AuthKind, Kind, CLOSE_WRITE, OPEN, OPEN_EXEC_PERM, OPEN_PERM, Q_OVERFLOW};

    fn sub(kinds: &[Kind], auth: &[AuthKind], muted: &[&str]) -> Subscription {
        Subscription { kinds: kinds.to_vec(), auth: auth.to_vec(), muted: muted.iter().map(|m| m.to_string()).collect() }
    }

    use bifrost_core::endpoint_security as es;

    fn ev(mask: u64, fd: i32, pid: i32) -> RawEvent {
        RawEvent { mask, fd, pid }
    }

    #[test]
    fn a_notification_reaches_only_the_subscribers_that_asked_and_polls_out_once() {
        let mut h = Hub::new();
        let a = h.subscribe("is.olibuijr.guard", sub(&[Kind::Open], &[], &[])).unwrap();
        let b = h.subscribe("is.olibuijr.other", sub(&[Kind::CloseWrite], &[], &["/home/skip"])).unwrap();
        assert_eq!(h.dispatch(&ev(OPEN, 9, 77), "/etc/passwd", 0.0), Dispatch::Notified);
        assert_eq!(h.dispatch(&ev(CLOSE_WRITE, 9, 78), "/home/skip/x", 0.0), Dispatch::Notified, "muted for b");
        let got = h.poll("is.olibuijr.guard", a, 10).unwrap();
        assert_eq!(got, [Delivered { kinds: vec![Kind::Open], auth: None, pid: 77, path: "/etc/passwd".into(), round: None }]);
        assert!(h.poll("is.olibuijr.guard", a, 10).unwrap().is_empty(), "taken once");
        assert!(h.poll("is.olibuijr.other", b, 10).unwrap().is_empty());
    }

    #[test]
    fn one_app_cannot_poll_or_end_another_apps_subscription() {
        let mut h = Hub::new();
        let a = h.subscribe("is.olibuijr.guard", sub(&[Kind::Open], &[], &[])).unwrap();
        assert!(h.poll("is.olibuijr.evil", a, 10).is_err());
        assert!(h.unsubscribe("is.olibuijr.evil", a).is_err());
        assert!(h.answer("is.olibuijr.evil", a, 1, Decision::Allow).is_err());
        assert_eq!(h.subscriptions(), 1);
    }

    #[test]
    fn subscriptions_are_limited_and_the_app_id_is_checked() {
        let mut h = Hub::new();
        for _ in 0..MAX_PER_APP {
            h.subscribe("is.olibuijr.guard", sub(&[Kind::Open], &[], &[])).unwrap();
        }
        assert!(h.subscribe("is.olibuijr.guard", sub(&[Kind::Open], &[], &[])).is_err(), "per app");
        let long = "x".repeat(129);
        for bad in ["", "a b", "../x", "a/b", long.as_str()] {
            assert!(h.subscribe(bad, sub(&[Kind::Open], &[], &[])).is_err(), "{bad}");
        }
        for i in 0..MAX_SUBSCRIPTIONS {
            let _ = h.subscribe(&format!("is.olibuijr.app{i}"), sub(&[Kind::Open], &[], &[]));
        }
        assert_eq!(h.subscriptions(), MAX_SUBSCRIPTIONS);
        assert!(h.subscribe("is.olibuijr.late", sub(&[Kind::Open], &[], &[])).is_err(), "in all");
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_counts_them() {
        let mut h = Hub::new();
        let a = h.subscribe("is.olibuijr.guard", sub(&[Kind::Open], &[], &[])).unwrap();
        for i in 0..(QUEUE_MAX + 3) {
            h.dispatch(&ev(OPEN, 5, i as i32), "/x", 0.0);
        }
        assert_eq!(h.dropped, 3);
        let first = h.poll("is.olibuijr.guard", a, POLL_MAX).unwrap();
        assert_eq!(first.len(), POLL_MAX);
        assert_eq!(first[0].pid, 3, "the three oldest are gone");
    }

    #[test]
    fn an_authorization_waits_for_every_subscriber_and_one_deny_denies() {
        let mut h = Hub::new();
        let a = h.subscribe("is.olibuijr.a", sub(&[], &[AuthKind::Exec], &[])).unwrap();
        let b = h.subscribe("is.olibuijr.b", sub(&[], &[AuthKind::Exec], &[])).unwrap();
        assert_eq!(h.dispatch(&ev(OPEN_EXEC_PERM, 11, 5), "/tmp/run", 0.0), Dispatch::Waiting);
        let ea = h.poll("is.olibuijr.a", a, 10).unwrap();
        let round = ea[0].round.expect("the app is told the round to answer");
        assert_eq!(ea[0].auth, Some(AuthKind::Exec));
        assert!(h.answer("is.olibuijr.a", a, round, Decision::Allow).unwrap().is_none(), "b has not answered");
        let done = h.answer("is.olibuijr.b", b, round, Decision::Deny).unwrap().expect("a deny ends the round");
        assert_eq!(done.fd, 11);
        assert_eq!(done.reply, es::response_bytes(11, Decision::Deny));
        assert!(h.answer("is.olibuijr.b", b, round, Decision::Allow).is_err(), "the round is over");
    }

    #[test]
    fn an_authorization_nobody_wants_is_allowed_at_once() {
        let mut h = Hub::new();
        h.subscribe("is.olibuijr.a", sub(&[Kind::Open], &[], &[])).unwrap();
        assert_eq!(h.dispatch(&ev(OPEN_PERM, 4, 5), "/x", 0.0), Dispatch::Answer(es::response_bytes(4, Decision::Allow)));
        let mut empty = Hub::new();
        assert_eq!(empty.dispatch(&ev(OPEN_EXEC_PERM, 4, 5), "/x", 0.0), Dispatch::Answer(es::response_bytes(4, Decision::Allow)));
    }

    #[test]
    fn a_stuck_subscriber_is_let_through_at_the_deadline() {
        let mut h = Hub::new();
        h.subscribe("is.olibuijr.a", sub(&[], &[AuthKind::Open], &[])).unwrap();
        assert_eq!(h.dispatch(&ev(OPEN_PERM, 6, 5), "/x", 10.0), Dispatch::Waiting);
        assert!(h.expire(10.0 + AUTH_DEADLINE_SECS - 0.1).is_empty());
        let late = h.expire(10.0 + AUTH_DEADLINE_SECS);
        assert_eq!(late, [Resolved { fd: 6, reply: es::response_bytes(6, Decision::Allow), timed_out: true }]);
        assert!(h.expire(99.0).is_empty(), "resolved once");
    }

    #[test]
    fn leaving_allows_what_the_subscriber_still_owed() {
        let mut h = Hub::new();
        let a = h.subscribe("is.olibuijr.a", sub(&[], &[AuthKind::Open], &[])).unwrap();
        assert_eq!(h.dispatch(&ev(OPEN_PERM, 8, 5), "/x", 0.0), Dispatch::Waiting);
        let done = h.unsubscribe_app("is.olibuijr.a");
        assert_eq!(done, [Resolved { fd: 8, reply: es::response_bytes(8, Decision::Allow), timed_out: false }]);
        assert_eq!(h.subscriptions(), 0);
        assert_eq!(h.mask(), 0);
        assert!(h.poll("is.olibuijr.a", a, 1).is_err());
    }

    #[test]
    fn the_mask_covers_what_the_subscribers_want() {
        let mut h = Hub::new();
        assert_eq!(h.mask(), 0);
        h.subscribe("is.olibuijr.a", sub(&[Kind::Open], &[AuthKind::Exec], &[])).unwrap();
        assert_eq!(h.mask(), OPEN | OPEN_EXEC_PERM);
    }

    #[test]
    fn an_overflow_record_is_not_an_event() {
        let o = ev(Q_OVERFLOW, -1, 0);
        assert!(o.overflowed());
    }

    #[test]
    fn real_file_systems_are_marked_once_per_device() {
        let info = "22 1 259:2 / / rw,relatime - btrfs /dev/nvme0n1p2 rw,ssd,subvolid=256\n\
                    23 22 0:21 / /proc rw - proc proc rw\n\
                    24 22 259:2 /home /home rw - btrfs /dev/nvme0n1p2 rw,subvolid=257\n\
                    25 22 259:1 / /boot rw - vfat /dev/nvme0n1p1 rw\n\
                    26 22 0:30 / /run rw - tmpfs tmpfs rw\n\
                    27 22 8:1 / /mnt/my\\040disk rw - ext4 /dev/sda1 rw\n";
        assert_eq!(fs_mounts(info), ["/", "/boot"]);
        assert!(fs_mounts("").is_empty());
    }

    #[test]
    fn subscribe_parameters_are_checked_before_a_descriptor_exists() {
        let j = |t: &str| bifrost_core::json::parse(t).unwrap();
        assert!(parse_subscribe(&j(r#"{"app":"is.olibuijr.guard","kinds":["open"]}"#)).is_ok());
        assert_eq!(parse_subscribe(&j(r#"{"kinds":["open"]}"#)).unwrap_err().code, "bad_params");
        assert_eq!(parse_subscribe(&j(r#"{"app":"a b","kinds":["open"]}"#)).unwrap_err().code, "bad_params");
        assert_eq!(parse_subscribe(&j(r#"{"app":"is.olibuijr.guard","kinds":["nope"]}"#)).unwrap_err().code, "bad_params");
        assert_eq!(parse_subscribe(&j(r#"{"app":"is.olibuijr.guard"}"#)).unwrap_err().code, "bad_params");
        assert!(handles(M_POLL) && !handles("system.ping"));
    }
}
