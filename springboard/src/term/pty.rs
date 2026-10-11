//! A pseudo-terminal with a child process on its slave side (bionic or glibc).

use crate::bare::sys::{check, close, fcntl, ioctl, ioctl_req, kill, open, read, waitpid, write};
use std::cell::Cell;
use std::ffi::{c_char, c_int, CString};
use std::io;
use std::ptr::null;
use super::identity::Login;

#[allow(non_camel_case_types)]
type pid_t = c_int;

const O_RDWR: c_int = 2;
const O_NOCTTY: c_int = 0o400;
const O_NONBLOCK: c_int = 0o4000;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;
const TIOCSWINSZ: u32 = 0x5414;
const TIOCSCTTY: u32 = 0x540e;
const WNOHANG: c_int = 1;
const SIGHUP: c_int = 1;
const SIGKILL: c_int = 9;
const SIGPIPE: c_int = 13;
const SIG_DFL: usize = 0;

#[repr(C)]
struct Winsize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

extern "C" {
    fn posix_openpt(flags: c_int) -> c_int;
    fn grantpt(fd: c_int) -> c_int;
    fn unlockpt(fd: c_int) -> c_int;
    fn ptsname_r(fd: c_int, buf: *mut c_char, len: usize) -> c_int;
    fn fork() -> pid_t;
    fn setsid() -> pid_t;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    fn _exit(code: c_int) -> !;
    fn signal(sig: c_int, handler: usize) -> usize;
    fn syscall(num: i64, ...) -> i64;
    fn setgroups(n: usize, groups: *const u32) -> c_int;
    fn setgid(gid: u32) -> c_int;
    fn setuid(uid: u32) -> c_int;
    fn chdir(path: *const c_char) -> c_int;
}

/// close_range(2) has the same number on every architecture this runs on.
const SYS_CLOSE_RANGE: i64 = 436;
const EINTR: i32 = 4;

fn cstrings(v: &[&str]) -> io::Result<Vec<CString>> {
    v.iter().map(|s| CString::new(*s).map_err(|_| io::ErrorKind::InvalidInput.into())).collect()
}

/// Null-terminated pointer array over `v` (which must outlive it).
fn ptrs(v: &[CString]) -> Vec<*const c_char> {
    v.iter().map(|s| s.as_ptr()).chain([null()]).collect()
}

pub struct Pty {
    master: c_int,
    pid: pid_t,
    exited: Cell<bool>,
}

impl Pty {
    /// Runs `argv` with environment `env` ("K=V") on a new pty of `cols` x `rows`.
    pub fn spawn(argv: &[&str], env: &[&str], cols: u16, rows: u16, login: Option<&Login>) -> io::Result<Pty> {
        // SAFETY: plain libc call, the fd is owned by the Pty below or closed on error.
        let master = check(unsafe { posix_openpt(O_RDWR | O_NOCTTY) })?;
        match Self::start(master, argv, env, cols, rows, login) {
            Ok(pid) => Ok(Pty { master, pid, exited: Cell::new(false) }),
            Err(e) => {
                // SAFETY: master is a valid fd we own.
                unsafe { close(master) };
                Err(e)
            }
        }
    }

    fn start(master: c_int, argv: &[&str], env: &[&str], cols: u16, rows: u16, login: Option<&Login>) -> io::Result<pid_t> {
        // SAFETY: master is a valid pty master fd.
        check(unsafe { grantpt(master) })?;
        // SAFETY: as above.
        check(unsafe { unlockpt(master) })?;
        let mut name = [0 as c_char; 64];
        // SAFETY: the buffer and its length are valid.
        let r = unsafe { ptsname_r(master, name.as_mut_ptr(), name.len()) };
        if r != 0 {
            return Err(if r > 0 { io::Error::from_raw_os_error(r) } else { io::Error::last_os_error() });
        }
        // SAFETY: fd is valid; F_SETFD takes an int argument.
        check(unsafe { fcntl(master, F_SETFD, FD_CLOEXEC) })?;
        // SAFETY: as above, F_GETFL takes no argument.
        let fl = check(unsafe { fcntl(master, F_GETFL) })?;
        // SAFETY: as above, F_SETFL takes an int argument.
        check(unsafe { fcntl(master, F_SETFL, fl | O_NONBLOCK) })?;
        set_size(master, cols, rows)?;

        // Everything the child touches is prepared before fork: it must not allocate.
        let (argv, env) = (cstrings(argv)?, cstrings(env)?);
        let (argp, envp) = (ptrs(&argv), ptrs(&env));
        let home = match login {
            Some(l) => Some(CString::new(l.home.to_string_lossy().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?),
            None => env.iter().find_map(|e| e.to_bytes().strip_prefix(b"HOME=").map(|h| CString::new(h).unwrap_or_default())),
        };
        let ids = login.map(|l| (l.uid, l.gid, l.groups.clone()));
        // grantpt ran as root: give the slave to the shell's user (group stays tty), or anything that reopens the
        // tty (/dev/tty, tmux, ssh, pinentry) fails after the privilege drop
        if let Some(l) = login {
            use std::os::unix::fs::PermissionsExt;
            // SAFETY: ptsname_r wrote a NUL-terminated path into `name`.
            let path = std::path::PathBuf::from(unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned());
            std::os::unix::fs::chown(&path, Some(l.uid), None)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o620))?;
        }
        // SAFETY: the child only calls async-signal-safe functions before execve/_exit.
        let pid = check(unsafe { fork() })?;
        if pid == 0 {
            // SAFETY: async-signal-safe calls on valid pointers; never returns.
            unsafe {
                setsid();
                let slave = open(name.as_ptr(), O_RDWR);
                if slave < 0 {
                    _exit(127);
                }
                ioctl(slave, TIOCSCTTY as ioctl_req, 0);
                for fd in 0..3 {
                    dup2(slave, fd);
                }
                if slave > 2 {
                    close(slave);
                }
                // nothing else of ours may leak into the shell (an old kernel just lacks the call)
                syscall(SYS_CLOSE_RANGE, 3i64, u32::MAX as i64, 0i64);
                signal(SIGPIPE, SIG_DFL);
                // drop privileges: groups, then gid, then uid; on failure never exec as root
                if let Some((uid, gid, groups)) = &ids {
                    if setgroups(groups.len(), groups.as_ptr()) != 0 || setgid(*gid) != 0 || setuid(*uid) != 0 {
                        _exit(126);
                    }
                }
                if let Some(h) = &home {
                    if chdir(h.as_ptr()) != 0 {
                        chdir(c"/".as_ptr());
                    }
                }
                execve(argp[0], argp.as_ptr(), envp.as_ptr());
                _exit(127)
            }
        }
        Ok(pid)
    }

    pub fn cwd(&self) -> Option<String> {
        std::fs::read_link(format!("/proc/{}/cwd", self.pid)).ok().map(|p| p.to_string_lossy().into_owned())
    }

    /// Name of the job in the foreground on this terminal ("vim", "ssh"), or None while
    /// the shell itself is in the foreground (at the prompt).
    pub fn job(&self) -> Option<String> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)).ok()?;
        let (pgrp, tpgid) = foreground_group(&stat)?;
        if tpgid <= 0 || tpgid == pgrp {
            return None;
        }
        let name = std::fs::read_to_string(format!("/proc/{tpgid}/comm")).ok()?;
        Some(name.trim().to_string()).filter(|n| !n.is_empty())
    }

    /// Pty master, for poll().
    pub fn fd(&self) -> c_int {
        self.master
    }

    /// Reads what is available; `Ok(0)` means the slave side is gone.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: buf is valid for buf.len() bytes.
        let n = unsafe { read(self.master, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            // Linux reports a hung-up slave as EIO on the master.
            return if e.raw_os_error() == Some(5) { Ok(0) } else { Err(e) };
        }
        Ok(n as usize)
    }

    /// Writes as much of `data` as the pty takes without blocking.
    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        // SAFETY: data is valid for data.len() bytes.
        let n = unsafe { write(self.master, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    /// Sets the terminal size; the child gets SIGWINCH.
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        set_size(self.master, cols, rows)
    }

    /// Whether the child is still running (reaps it once it has exited).
    pub fn alive(&self) -> bool {
        if !self.exited.get() {
            let mut status = 0;
            // SAFETY: status is a valid out pointer; WNOHANG never blocks.
            let r = unsafe { waitpid(self.pid, &mut status, WNOHANG) };
            self.exited.set(r == self.pid || r < 0);
        }
        !self.exited.get()
    }
}

/// The process group (field 5) and the terminal's foreground group (field 8) of a
/// `/proc/<pid>/stat` line. Fields are counted after the command name in parentheses,
/// which may contain spaces.
fn foreground_group(stat: &str) -> Option<(i64, i64)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // state, ppid, pgrp, session, tty_nr, tpgid
    let mut fields = rest.split_whitespace().skip(2);
    let pgrp = fields.next()?.parse().ok()?;
    let tpgid = fields.nth(2)?.parse().ok()?;
    Some((pgrp, tpgid))
}

fn set_size(fd: c_int, cols: u16, rows: u16) -> io::Result<()> {
    let mut ws = Winsize { rows, cols, xpixel: 0, ypixel: 0 };
    // SAFETY: TIOCSWINSZ reads a struct winsize from a valid pointer.
    check(unsafe { ioctl(fd, TIOCSWINSZ as ioctl_req, &mut ws as *mut Winsize) }).map(drop)
}

/// One non-blocking wait on `pid`: true once it is reaped (or not ours any more).
fn try_reap(pid: pid_t) -> bool {
    loop {
        // SAFETY: a null status pointer is allowed; WNOHANG never blocks.
        let r = unsafe { waitpid(pid, std::ptr::null_mut(), WNOHANG) };
        if r < 0 && io::Error::last_os_error().raw_os_error() == Some(EINTR) {
            continue;
        }
        return r != 0;
    }
}

/// Reaps a killed child without ever stalling the caller: a few short tries,
/// then a detached thread finishes the job.
fn reap(pid: pid_t) {
    for _ in 0..10 {
        if try_reap(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    crate::bare::sys::spawn_named("pty-reap", move || {
        while !try_reap(pid) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.alive() {
            // SAFETY: signalling our own child.
            unsafe {
                kill(self.pid, SIGHUP);
                kill(self.pid, SIGKILL);
            }
            reap(self.pid);
        }
        // SAFETY: we own the master fd.
        unsafe { close(self.master) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(p: &Pty) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 5 {
            crate::bare::sys::wait(&[p.fd()], 100);
            match p.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            }
        }
        out
    }

    #[test]
    fn foreground_group_reads_pgrp_and_tpgid_past_a_spaced_name() {
        assert_eq!(foreground_group("1234 (my shell) S 1 1234 1234 34816 4321 4194304 1 2"), Some((1234, 4321)));
        assert_eq!(foreground_group("1234 (bash) S 1 1234 1234 34816 1234"), Some((1234, 1234)));
        assert_eq!(foreground_group("garbage"), None);
    }

    #[test]
    fn child_sees_a_tty_with_our_size_and_env() {
        let p = Pty::spawn(&["/bin/sh", "-c", "stty size; echo $TERM; [ -t 0 ] && echo tty"], &["TERM=xterm-256color"], 80, 24, None).unwrap();
        let out = String::from_utf8(drain(&p)).unwrap();
        assert!(out.contains("24 80"), "{out:?}");
        assert!(out.contains("xterm-256color"), "{out:?}");
        assert!(out.contains("tty"), "{out:?}");
        let start = std::time::Instant::now();
        while p.alive() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!p.alive());
    }

    #[test]
    fn the_child_inherits_no_stray_descriptors() {
        // a descriptor without CLOEXEC, as some other part of the shell might leave behind
        let null = CString::new("/dev/null").unwrap();
        // SAFETY: valid path; F_DUPFD (0) takes a minimum fd number and clears CLOEXEC.
        let fd = unsafe { open(null.as_ptr(), O_RDWR) };
        let high = unsafe { fcntl(fd, 0, 50) };
        assert!(high >= 50);
        let p = Pty::spawn(&["/bin/sh", "-c", "[ -e /proc/self/fd/$0 ] && echo leaked || echo clean", &high.to_string()], &[], 80, 24, None).unwrap();
        let out = String::from_utf8(drain(&p)).unwrap();
        unsafe {
            close(fd);
            close(high);
        }
        assert!(out.contains("clean"), "{out:?}");
    }

    #[test]
    fn dropping_a_pty_kills_and_reaps_a_stubborn_child() {
        let p = Pty::spawn(&["/bin/sh", "-c", "trap '' HUP; sleep 30"], &[], 80, 24, None).unwrap();
        let pid = p.pid;
        let start = std::time::Instant::now();
        drop(p);
        assert!(start.elapsed().as_secs() < 2, "drop must not stall");
        let t = std::time::Instant::now();
        // SAFETY: probing our own (reaped or about to be reaped) child with signal 0.
        while unsafe { kill(pid, 0) } == 0 && t.elapsed().as_secs() < 3 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(unsafe { kill(pid, 0) } != 0, "the child was reaped");
    }

    #[test]
    fn missing_program_exits_and_input_reaches_the_child() {
        let p = Pty::spawn(&["/nonexistent"], &[], 80, 24, None).unwrap();
        drain(&p);
        let p = Pty::spawn(&["/bin/sh", "-c", "read x; echo got:$x"], &[], 80, 24, None).unwrap();
        p.write(b"abc\n").unwrap();
        assert!(String::from_utf8(drain(&p)).unwrap().contains("got:abc"));
    }
}
