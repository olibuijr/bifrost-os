//! Game Mode's priority request: `bifrostd` raises a game's scheduling priority (nice and the uclamp minimum) and puts
//! it back, for the process the shell started and holds in front.
//!
//! Wire format, one line per request on `<root>/run/priority.sock`: `priority <pid> <nice> <util_min>`. The reply is
//! `ok` or `err <reason>`. `nice` must be between -5 and 0 and `util_min` between 0 and 1024 (0 clears the hint, and 0
//! puts nice back to 0 when the game leaves the front).
//!
//! Who may ask: the peer of the socket (`SO_PEERCRED`) may only name its own child, owned by the same user, whose
//! executable lives under `<root>/apps/` (an installed app). Every thread of that process gets the setting, because
//! nice and uclamp are per thread on Linux. Nothing else is accepted, so the request cannot reach other processes.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// The most negative nice a game may get.
pub const NICE_MIN: i32 = -5;
/// The largest uclamp minimum (the kernel's scale).
pub const UTIL_MAX: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub pid: u32,
    pub nice: i32,
    pub util_min: u32,
}

/// Parses `priority <pid> <nice> <util_min>`, checking the ranges.
pub fn parse(line: &str) -> Result<Request, String> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("priority") {
        return Err("unknown request".into());
    }
    let pid: u32 = parts.next().and_then(|s| s.parse().ok()).filter(|&p| p > 1).ok_or("bad pid")?;
    let nice: i32 = parts.next().and_then(|s| s.parse().ok()).ok_or("bad nice")?;
    let util_min: u32 = parts.next().and_then(|s| s.parse().ok()).ok_or("bad util_min")?;
    if parts.next().is_some() {
        return Err("too many fields".into());
    }
    if !(NICE_MIN..=0).contains(&nice) {
        return Err("nice out of range".into());
    }
    if util_min > UTIL_MAX {
        return Err("util_min out of range".into());
    }
    Ok(Request { pid, nice, util_min })
}

/// The parent pid in a `/proc/<pid>/stat` line. The command name is in parentheses and may hold spaces.
pub fn stat_ppid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // fields after the name: state, ppid, ...
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// The check that decides whether `peer` may change `target`: a child of the peer, of the same user, running an app
/// installed under `apps`. Returns the reason it is refused.
pub fn allowed(target_ppid: u32, target_uid: u32, target_exe: &Path, peer_pid: u32, peer_uid: u32, apps: &Path) -> Result<(), &'static str> {
    if target_ppid != peer_pid {
        return Err("not a child of the caller");
    }
    if target_uid != peer_uid {
        return Err("another user's process");
    }
    if !target_exe.starts_with(apps) {
        return Err("not an installed app");
    }
    Ok(())
}

/// `struct sched_attr` (linux/sched/types.h), SCHED_ATTR_SIZE_VER1.
#[repr(C)]
#[derive(Default)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;
const PRIO_PROCESS: i32 = 0;
const SOL_SOCKET: i32 = 1;
const SO_PEERCRED: i32 = 17;

#[cfg(target_arch = "x86_64")]
const SYS_SCHED_SETATTR: i64 = 314;
#[cfg(target_arch = "aarch64")]
const SYS_SCHED_SETATTR: i64 = 274;

#[repr(C)]
#[derive(Default)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

extern "C" {
    fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    fn syscall(num: i64, ...) -> i64;
    fn getsockopt(fd: i32, level: i32, name: i32, val: *mut u8, len: *mut u32) -> i32;
}

/// Sets nice and the uclamp minimum of thread `tid`.
fn apply_thread(tid: u32, nice: i32, util_min: u32) -> std::io::Result<()> {
    // SAFETY: setpriority takes plain integers.
    if unsafe { setpriority(PRIO_PROCESS, tid, nice) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_flags: SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN | SCHED_FLAG_UTIL_CLAMP_MAX,
        sched_util_min: util_min.min(UTIL_MAX),
        sched_util_max: UTIL_MAX,
        ..SchedAttr::default()
    };
    // SAFETY: `attr` is a sched_attr of the size it declares, alive for the call.
    let r = unsafe { syscall(SYS_SCHED_SETATTR, tid as i32, &attr as *const SchedAttr, 0u32) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Applies a checked request to every thread of the process.
pub fn apply(req: &Request) -> Result<usize, String> {
    let mut done = 0;
    let tasks = std::fs::read_dir(format!("/proc/{}/task", req.pid)).map_err(|e| format!("no such process: {e}"))?;
    for entry in tasks.flatten() {
        let Some(tid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        apply_thread(tid, req.nice, req.util_min).map_err(|e| format!("thread {tid}: {e}"))?;
        done += 1;
    }
    Ok(done)
}

/// The user id and parent pid of `pid`, read from `/proc`.
fn proc_owner(pid: u32) -> Option<(u32, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ppid = stat_ppid(&stat)?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let uid = status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()?;
    Some((ppid, uid))
}

/// The pid and uid of the process on the other end of `stream`.
fn peer(stream: &UnixStream) -> Option<(u32, u32)> {
    let mut cred = Ucred::default();
    let mut len = std::mem::size_of::<Ucred>() as u32;
    // SAFETY: the buffer and length describe a Ucred, which is what SO_PEERCRED writes.
    let r = unsafe { getsockopt(stream.as_raw_fd(), SOL_SOCKET, SO_PEERCRED, &mut cred as *mut Ucred as *mut u8, &mut len) };
    (r == 0 && cred.pid > 0).then_some((cred.pid as u32, cred.uid))
}

/// Answers one request from `stream`: checks the peer and the target, applies, and writes the reply line.
fn handle(stream: &mut UnixStream, apps: &Path) -> String {
    let Some((peer_pid, peer_uid)) = peer(stream) else { return "err no peer".into() };
    // a peer that never finishes its line does not hold the queue
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let Ok(clone) = stream.try_clone() else { return "err clone".into() };
    let mut line = String::new();
    if BufReader::new(clone).read_line(&mut line).is_err() {
        return "err read".into();
    }
    let req = match parse(&line) {
        Ok(r) => r,
        Err(e) => return format!("err {e}"),
    };
    let Some((ppid, uid)) = proc_owner(req.pid) else { return "err no such process".into() };
    let exe = std::fs::read_link(format!("/proc/{}/exe", req.pid)).unwrap_or_default();
    if let Err(e) = allowed(ppid, uid, &exe, peer_pid, peer_uid, apps) {
        return format!("err {e}");
    }
    match apply(&req) {
        Ok(_) => "ok".into(),
        Err(e) => format!("err {e}"),
    }
}

/// Serves requests on `socket` until the process ends. Requests are answered one at a time, so a slow peer only waits
/// for itself; each reply is written before the next connection is taken.
pub fn serve(socket: PathBuf, apps: PathBuf) {
    let _ = std::fs::remove_file(&socket);
    let Ok(listener) = UnixListener::bind(&socket) else { return };
    for mut stream in listener.incoming().flatten() {
        let reply = handle(&mut stream, &apps);
        let _ = writeln!(stream, "{reply}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_parses_and_its_ranges_are_checked() {
        assert_eq!(parse("priority 4242 -5 512\n"), Ok(Request { pid: 4242, nice: -5, util_min: 512 }));
        assert_eq!(parse("priority 4242 0 0"), Ok(Request { pid: 4242, nice: 0, util_min: 0 }), "leaving puts it back");
        assert!(parse("priority 4242 -6 0").is_err(), "nice below the floor");
        assert!(parse("priority 4242 1 0").is_err(), "a lower priority is not asked for");
        assert!(parse("priority 4242 0 1025").is_err());
        assert!(parse("priority 1 -5 0").is_err(), "init is never a target");
        assert!(parse("priority 4242 -5").is_err());
        assert!(parse("priority 4242 -5 0 extra").is_err());
        assert!(parse("shutdown 4242 -5 0").is_err());
    }

    #[test]
    fn the_parent_pid_is_read_past_a_name_with_spaces() {
        assert_eq!(stat_ppid("4242 (my game) S 1234 4242 4242 0 -1"), Some(1234));
        assert_eq!(stat_ppid("4242 (a)) S 77 1 1 0"), Some(77), "the last parenthesis ends the name");
        assert_eq!(stat_ppid("garbage"), None);
    }

    #[test]
    fn only_an_installed_child_of_the_same_user_may_be_changed() {
        let apps = Path::new("/var/lib/springboard/apps");
        let exe = Path::new("/var/lib/springboard/apps/is.example.game/current/bin/game");
        assert_eq!(allowed(100, 1000, exe, 100, 1000, apps), Ok(()));
        assert_eq!(allowed(101, 1000, exe, 100, 1000, apps), Err("not a child of the caller"));
        assert_eq!(allowed(100, 0, exe, 100, 1000, apps), Err("another user's process"));
        let stray = Path::new("/usr/bin/sleep");
        assert_eq!(allowed(100, 1000, stray, 100, 1000, apps), Err("not an installed app"));
    }
}
