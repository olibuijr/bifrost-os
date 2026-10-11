//! The handful of libc calls the bare-metal shell needs (bionic or glibc).
#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void, CString};
use std::io;

pub type ioctl_req = std::ffi::c_ulong;
type nfds_t = std::ffi::c_ulong;

#[repr(C)]
pub struct pollfd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

#[repr(C)]
pub struct timeval {
    pub tv_sec: c_long,
    pub tv_usec: c_long,
}

#[repr(C)]
pub struct timespec {
    pub tv_sec: i64,
    pub tv_nsec: c_long,
}

#[repr(C)]
pub struct tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: c_long,
    pub tm_zone: *const c_char,
}

pub const O_RDWR: c_int = 2;
pub const O_RDONLY: c_int = 0;
pub const O_NONBLOCK: c_int = 0o4000;
pub const O_CLOEXEC: c_int = 0o2000000;
pub const POLLIN: i16 = 1;
pub const POLLERR: i16 = 8;
pub const POLLHUP: i16 = 0x10;
pub const PROT_RW: c_int = 3;
pub const MAP_SHARED: c_int = 1;
pub const CLOCK_MONOTONIC: c_int = 1;
pub const CLOCK_BOOTTIME: c_int = 7;
pub const CLOCK_REALTIME: c_int = 0;

extern "C" {
    pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    pub fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    pub fn sysconf(name: c_int) -> i64;
    pub fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int;
    pub fn bind(fd: c_int, addr: *const u8, len: c_uint) -> c_int;
    pub fn connect(fd: c_int, addr: *const u8, len: c_uint) -> c_int;
    pub fn send(fd: c_int, buf: *const u8, len: usize, flags: c_int) -> isize;
    pub fn recv(fd: c_int, buf: *mut u8, len: usize, flags: c_int) -> isize;
    pub fn setsockopt(fd: c_int, level: c_int, name: c_int, val: *const c_void, len: c_uint) -> c_int;
    pub fn if_nametoindex(name: *const c_char) -> c_uint;
    pub fn ioctl(fd: c_int, req: ioctl_req, ...) -> c_int;
    pub fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn poll(fds: *mut pollfd, n: nfds_t, timeout: c_int) -> c_int;
    fn clock_gettime(clk: c_int, ts: *mut timespec) -> c_int;
    fn clock_settime(clk: c_int, ts: *const timespec) -> c_int;
    pub fn time(t: *mut i64) -> i64;
    pub fn localtime_r(t: *const i64, out: *mut tm) -> *mut tm;
    fn tzset();
    pub fn kill(pid: c_int, sig: c_int) -> c_int;
}

/// `std::thread::spawn` with a thread name, so per-thread numbers in /proc and top are attributable.
pub fn spawn_named<F, T>(name: &str, f: F) -> std::thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new().name(name.into()).spawn(f).expect("failed to spawn thread")
}

/// Maps a libc return of -1 to the OS error.
pub fn check(r: c_int) -> io::Result<c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

/// `struct statvfs` on 64-bit glibc and bionic: eleven `unsigned long`
/// fields followed by spare space.
#[repr(C)]
struct statvfs_t {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    _spare: [u64; 8],
}

extern "C" {
    fn statvfs(path: *const c_char, buf: *mut statvfs_t) -> c_int;
}

/// (total, available) bytes of the filesystem holding `path`.
pub fn disk_space(path: &str) -> Option<(u64, u64)> {
    let p = CString::new(path).ok()?;
    // SAFETY: `p` is NUL-terminated and `st` is a zeroed struct at least as
    // large as the platform's `struct statvfs`.
    let mut st: statvfs_t = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(p.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = if st.f_frsize > 0 { st.f_frsize } else { st.f_bsize };
    Some((st.f_blocks * unit, st.f_bavail * unit))
}

pub fn open_dev(path: &str, flags: c_int) -> io::Result<c_int> {
    let p = CString::new(path).map_err(|_| io::ErrorKind::InvalidInput)?;
    let fd = unsafe { open(p.as_ptr(), flags | O_CLOEXEC) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

/// ioctl with a pointer argument; maps -1 to the OS error.
///
/// # Safety
/// `arg` must point to writable memory at least as large as the size encoded in `req`, valid for the call.
pub unsafe fn ioctl_ptr<T>(fd: c_int, req: u32, arg: *mut T) -> io::Result<()> {
    // SAFETY: the caller guarantees `arg` covers the size `req` encodes.
    if unsafe { ioctl(fd, req as ioctl_req, arg) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    dir << 30 | (size as u32) << 16 | (ty as u32) << 8 | nr as u32
}
pub fn iowr<T>(ty: u8, nr: u8) -> u32 {
    ioc(3, ty, nr, std::mem::size_of::<T>())
}
pub fn iow<T>(ty: u8, nr: u8) -> u32 {
    ioc(1, ty, nr, std::mem::size_of::<T>())
}
pub fn io(ty: u8, nr: u8) -> u32 {
    ioc(0, ty, nr, 0)
}

/// Waits for input on `fds`; returns which ones are readable. A hung-up or
/// failed fd counts as readable, since a read on it returns at once.
pub fn wait(fds: &[c_int], timeout_ms: i32) -> Vec<bool> {
    let mut out = Vec::with_capacity(fds.len());
    wait_into(fds, timeout_ms, &mut out);
    out
}

/// `wait` into a caller-kept vector (cleared first); the pollfd scratch is per thread, so a
/// loop that calls this every wake allocates nothing.
pub fn wait_into(fds: &[c_int], timeout_ms: i32, out: &mut Vec<bool>) {
    thread_local! {
        static SCRATCH: std::cell::RefCell<Vec<pollfd>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    SCRATCH.with(|s| {
        let mut p = s.borrow_mut();
        p.clear();
        p.extend(fds.iter().map(|&fd| pollfd { fd, events: POLLIN, revents: 0 }));
        unsafe { poll(p.as_mut_ptr(), p.len() as nfds_t, timeout_ms) };
        out.clear();
        out.extend(p.iter().map(|f| f.revents & (POLLIN | POLLHUP | POLLERR) != 0));
    });
}

pub fn now_ms() -> f64 {
    let mut ts = timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 * 1e3 + ts.tv_nsec as f64 / 1e6
}

/// CLOCK_BOOTTIME in ms: like `now_ms` but it keeps counting while the device is suspended.
/// `boottime_ms() - now_ms()` is the total time spent asleep so far.
pub fn boottime_ms() -> f64 {
    let mut ts = timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec as f64 * 1e3 + ts.tv_nsec as f64 / 1e6
}

/// Local wall-clock time as (hour, min, sec, wday, mday, mon), in the zone
/// `timezone` selected (libc's zone when no tz database is readable).
pub fn local_time() -> (i32, i32, i32, i32, i32, i32) {
    let now = unsafe { time(std::ptr::null_mut()) };
    if let Some(off) = super::timezone::offset_at(now) {
        return super::timezone::civil(now + off as i64);
    }
    unsafe {
        tzset();
        let mut t: tm = std::mem::zeroed();
        localtime_r(&now, &mut t);
        (t.tm_hour, t.tm_min, t.tm_sec, t.tm_wday, t.tm_mday, t.tm_mon)
    }
}

/// Unix time in seconds and the local zone's offset from UTC in seconds.
pub fn wall_time() -> (i64, i32) {
    let now = unsafe { time(std::ptr::null_mut()) };
    if let Some(off) = super::timezone::offset_at(now) {
        return (now, off);
    }
    unsafe {
        tzset();
        let mut t: tm = std::mem::zeroed();
        localtime_r(&now, &mut t);
        (now, t.tm_gmtoff as i32)
    }
}

/// Sets the system clock (CLOCK_REALTIME) to `sec` seconds and `nsec`
/// nanoseconds since the epoch; needs root.
pub fn set_realtime(sec: i64, nsec: i64) -> io::Result<()> {
    let ts = timespec { tv_sec: sec, tv_nsec: nsec as c_long };
    // SAFETY: `ts` is a valid timespec for the duration of the call.
    if unsafe { clock_settime(CLOCK_REALTIME, &ts) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Android system properties do not exist here; kept so callers need no change. A no-op.
pub fn set_prop(_key: &str, _value: &str) {}

/// Android system properties do not exist here; always empty.
pub fn get_prop(_key: &str) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_space_reads_the_root_filesystem() {
        let (total, free) = disk_space("/").unwrap();
        assert!(total > 0 && free <= total);
        assert_eq!(disk_space("/nonexistent/path"), None);
    }

    #[test]
    fn ioctl_numbers_match_kernel_headers() {
        // DRM_IOCTL_SET_MASTER = _IO('d', 0x1e)
        assert_eq!(io(b'd', 0x1e), 0x641e);
        // EVIOCGRAB = _IOW('E', 0x90, int)
        assert_eq!(iow::<c_int>(b'E', 0x90), 0x4004_4590);
        // DRM_IOCTL_MODE_CREATE_DUMB = _IOWR('d', 0xb2, 32-byte struct)
        assert_eq!(iowr::<[u8; 32]>(b'd', 0xb2), 0xc020_64b2);
    }
}

/// Maps a file read-only for the life of the process (fonts, wallpapers): the pages come from the
/// page cache, so every process mapping the same file shares one copy and untouched parts cost nothing.
pub use bifrost_render::assets::map_file;
