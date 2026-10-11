//! WatchEvents parity: `WatchStream` over inotify (recursive, new subfolders are watched as they appear).
//!
//! No threads and no polling. The stream owns one non-blocking inotify descriptor; the app returns `fd()` from
//! `App::wake_fd` and calls `poll()` from `tick`. Events are coalesced per path over `latency` and handed back on
//! the calling (main) thread; `next_deadline()` feeds `App::next_wake` while a batch is still maturing.
//! `NO_DEFER` delivers the first event after a quiet period at once, then coalesces the rest; `FILE_EVENTS` is
//! accepted for parity (events are always per file here).

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
    fn read(fd: c_int, buf: *mut u8, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
}
const IN_NONBLOCK: c_int = 0o4000;
const IN_CLOEXEC: c_int = 0o2000000;
const IN_MODIFY: u32 = 0x2;
const IN_CLOSE_WRITE: u32 = 0x8;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_DELETE_SELF: u32 = 0x400;
const IN_Q_OVERFLOW: u32 = 0x4000;
const IN_IGNORED: u32 = 0x8000;
const IN_ISDIR: u32 = 0x4000_0000;
const MASK: u32 = IN_MODIFY | IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE | IN_DELETE_SELF;

/// `WatchStreamEventFlags`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WatchEventFlags(pub u32);
impl WatchEventFlags {
    pub const CREATED: Self = Self(1);
    pub const REMOVED: Self = Self(2);
    pub const RENAMED: Self = Self(4);
    pub const MODIFIED: Self = Self(8);
    pub const ITEM_IS_DIR: Self = Self(16);
    pub const MUST_SCAN_SUB_DIRS: Self = Self(32);
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
}

/// `WatchStreamCreateFlags`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchStreamFlags(pub u32);
impl WatchStreamFlags {
    pub const NONE: Self = Self(0);
    pub const NO_DEFER: Self = Self(2);
    pub const FILE_EVENTS: Self = Self(16);
    /// BifrOSt extension (no counterpart): watch only the roots themselves, not their subfolders.     /// WatchEvents is recursive at no cost because the kernel journals every volume; over inotify a recursive stream
    /// walks the whole tree on `start`, which is too slow for `/` or a home folder on the UI thread.
    pub const SHALLOW: Self = Self(1 << 16);
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    const fn has(self, o: Self) -> bool {
        self.0 & o.0 != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub flags: WatchEventFlags,
}

pub struct WatchStream {
    fd: c_int,
    roots: Vec<PathBuf>,
    wds: HashMap<c_int, PathBuf>,
    latency: Duration,
    flags: WatchStreamFlags,
    running: bool,
    /// Coalesced events in first-seen order, and when the batch is due.
    pending: Vec<WatchEvent>,
    due: Option<Instant>,
    /// When the last delivery happened (for `NO_DEFER`).
    last_delivery: Option<Instant>,
}

impl WatchStream {
    /// A stopped stream over `paths` (folders, watched recursively). Call `start()`.
    pub fn new(paths: &[PathBuf], latency: Duration, flags: WatchStreamFlags) -> Self {
        Self {
            fd: -1,
            roots: paths.to_vec(),
            wds: HashMap::new(),
            latency,
            flags,
            running: false,
            pending: Vec::new(),
            due: None,
            last_delivery: None,
        }
    }

    /// Opens the descriptor and watches every folder under the roots. False when inotify is unavailable.
    pub fn start(&mut self) -> bool {
        if self.running {
            return true;
        }
        // SAFETY: plain syscall.
        let fd = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
        if fd < 0 {
            return false;
        }
        self.fd = fd;
        for r in self.roots.clone() {
            self.add_tree(&r);
        }
        self.running = true;
        true
    }

    pub fn stop(&mut self) {
        if self.fd >= 0 {
            // SAFETY: closing our own descriptor; the kernel drops its watches.
            unsafe { close(self.fd) };
        }
        self.fd = -1;
        self.wds.clear();
        self.pending.clear();
        self.due = None;
        self.running = false;
    }

    /// The descriptor for `App::wake_fd` (None while stopped).
    pub fn fd(&self) -> Option<i32> {
        (self.fd >= 0).then_some(self.fd)
    }

    /// Time until a maturing batch is due, for `App::next_wake`.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.due.map(|d| d.saturating_duration_since(now))
    }

    fn add_tree(&mut self, dir: &Path) {
        if self.flags.has(WatchStreamFlags::SHALLOW) {
            // the roots only: a subfolder created later is reported but not watched
            if self.roots.iter().any(|r| r == dir) {
                self.add_watch(dir);
            }
            return;
        }
        self.add_watch(dir);
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    self.add_tree(&e.path());
                }
            }
        }
    }

    fn add_watch(&mut self, dir: &Path) {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return };
        // SAFETY: `c` is a live NUL-terminated string for the call.
        let wd = unsafe { inotify_add_watch(self.fd, c.as_ptr(), MASK) };
        if wd >= 0 {
            self.wds.insert(wd, dir.to_path_buf());
        }
    }

    fn push(&mut self, path: PathBuf, flags: WatchEventFlags) {
        if let Some(e) = self.pending.iter_mut().find(|e| e.path == path) {
            e.flags = e.flags.union(flags);
        } else {
            self.pending.push(WatchEvent { path, flags });
        }
    }

    /// Reads queued kernel events and returns the coalesced batch once `latency` has passed since the first
    /// pending event (at once for the first event after a quiet period with `NO_DEFER`). Call from `tick`.
    pub fn poll(&mut self, now: Instant) -> Vec<WatchEvent> {
        if self.fd < 0 {
            return Vec::new();
        }
        let mut buf = [0u8; 8192];
        loop {
            // SAFETY: reads into a live buffer from our own non-blocking descriptor.
            let n = unsafe { read(self.fd, buf.as_mut_ptr(), buf.len()) };
            if n <= 0 {
                break;
            }
            let n = n as usize;
            let mut off = 0usize;
            while off + 16 <= n {
                let wd = i32::from_ne_bytes(buf[off..off + 4].try_into().unwrap());
                let mask = u32::from_ne_bytes(buf[off + 4..off + 8].try_into().unwrap());
                let len = u32::from_ne_bytes(buf[off + 12..off + 16].try_into().unwrap()) as usize;
                let raw = &buf[off + 16..(off + 16 + len).min(n)];
                let name = raw.split(|b| *b == 0).next().unwrap_or(&[]);
                off += 16 + len;
                if mask & IN_Q_OVERFLOW != 0 {
                    for r in self.roots.clone() {
                        self.push(r, WatchEventFlags::MUST_SCAN_SUB_DIRS.union(WatchEventFlags::ITEM_IS_DIR));
                    }
                    continue;
                }
                if mask & IN_IGNORED != 0 {
                    self.wds.remove(&wd);
                    continue;
                }
                let Some(dir) = self.wds.get(&wd).cloned() else { continue };
                let path = if name.is_empty() { dir } else { dir.join(std::ffi::OsStr::from_bytes(name)) };
                let mut f = WatchEventFlags::default();
                if mask & IN_CREATE != 0 {
                    f = f.union(WatchEventFlags::CREATED);
                }
                if mask & (IN_DELETE | IN_DELETE_SELF) != 0 {
                    f = f.union(WatchEventFlags::REMOVED);
                }
                if mask & (IN_MOVED_FROM | IN_MOVED_TO) != 0 {
                    f = f.union(WatchEventFlags::RENAMED);
                }
                if mask & (IN_MODIFY | IN_CLOSE_WRITE) != 0 {
                    f = f.union(WatchEventFlags::MODIFIED);
                }
                if mask & IN_ISDIR != 0 {
                    f = f.union(WatchEventFlags::ITEM_IS_DIR);
                    if mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                        // a new subfolder: watch it, and flag a rescan for what landed before the watch existed
                        self.add_tree(&path);
                        f = f.union(WatchEventFlags::MUST_SCAN_SUB_DIRS);
                    }
                }
                self.push(path, f);
            }
        }
        if self.pending.is_empty() {
            return Vec::new();
        }
        let due = match self.due {
            Some(d) => d,
            None => {
                let quiet = self.last_delivery.map_or(true, |t| now.saturating_duration_since(t) >= self.latency);
                let d = if self.flags.has(WatchStreamFlags::NO_DEFER) && quiet { now } else { now + self.latency };
                self.due = Some(d);
                d
            }
        };
        if now >= due {
            self.due = None;
            self.last_delivery = Some(now);
            std::mem::take(&mut self.pending)
        } else {
            Vec::new()
        }
    }
}

impl Drop for WatchStream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(s: &mut WatchStream, want: impl Fn(&[WatchEvent]) -> bool) -> Vec<WatchEvent> {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            all.extend(s.poll(Instant::now()));
            if want(&all) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    #[test]
    fn shallow_watches_only_the_roots() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join("old");
        std::fs::create_dir(&old).unwrap();
        let flags = WatchStreamFlags::FILE_EVENTS.union(WatchStreamFlags::NO_DEFER).union(WatchStreamFlags::SHALLOW);
        let mut s = WatchStream::new(&[t.path().to_path_buf()], Duration::from_millis(20), flags);
        assert!(s.start());
        assert_eq!(s.wds.len(), 1, "only the root is watched");
        let d = t.path().join("sub");
        std::fs::create_dir(&d).unwrap();
        let ev = wait(&mut s, |e| e.iter().any(|e| e.path == d));
        assert!(ev.iter().any(|e| e.path == d), "a new subfolder of the root is reported");
        assert_eq!(s.wds.len(), 1, "and not watched");
    }

    #[test]
    fn reports_create_remove_and_watches_new_subdirs() {
        let t = tempfile::tempdir().unwrap();
        let mut s = WatchStream::new(
            &[t.path().to_path_buf()],
            Duration::from_millis(20),
            WatchStreamFlags::FILE_EVENTS.union(WatchStreamFlags::NO_DEFER),
        );
        assert!(s.start());
        assert!(s.fd().is_some());
        let f = t.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let ev = wait(&mut s, |e| e.iter().any(|e| e.path == f && e.flags.contains(WatchEventFlags::MODIFIED)));
        let e = ev.iter().find(|e| e.path == f).expect("event for the new file");
        assert!(e.flags.contains(WatchEventFlags::CREATED));

        std::fs::remove_file(&f).unwrap();
        let ev = wait(&mut s, |e| e.iter().any(|e| e.flags.contains(WatchEventFlags::REMOVED)));
        assert!(ev.iter().any(|e| e.path == f && e.flags.contains(WatchEventFlags::REMOVED)));

        let d = t.path().join("sub");
        std::fs::create_dir(&d).unwrap();
        let ev = wait(&mut s, |e| e.iter().any(|e| e.path == d));
        assert!(ev.iter().any(|e| e.path == d && e.flags.contains(WatchEventFlags::ITEM_IS_DIR)));
        let g = d.join("b.txt");
        std::fs::write(&g, b"y").unwrap();
        let ev = wait(&mut s, |e| e.iter().any(|e| e.path == g));
        assert!(ev.iter().any(|e| e.path == g), "a new subfolder is watched");

        s.stop();
        assert!(s.fd().is_none());
    }
}
