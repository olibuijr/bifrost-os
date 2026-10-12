//! Thin syscall helpers: ioctl number encoding (the Linux `_IOC` layout of x86-64 and ARM64), the ioctl call,
//! device open and the monotonic clocks. The only place `libc::ioctl` is called.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

pub const DIR_NONE: u32 = 0;
pub const DIR_WRITE: u32 = 1;
pub const DIR_READ: u32 = 2;

/// `_IOC(dir, ty, nr, size)`: nr in bits 0..8, type 8..16, size 16..30, direction 30..32.
pub const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}
pub const fn io(ty: u8, nr: u8) -> u32 {
    ioc(DIR_NONE, ty, nr, 0)
}
pub const fn ior<T>(ty: u8, nr: u8) -> u32 {
    ioc(DIR_READ, ty, nr, std::mem::size_of::<T>())
}
pub const fn iow<T>(ty: u8, nr: u8) -> u32 {
    ioc(DIR_WRITE, ty, nr, std::mem::size_of::<T>())
}
pub const fn iowr<T>(ty: u8, nr: u8) -> u32 {
    ioc(DIR_READ | DIR_WRITE, ty, nr, std::mem::size_of::<T>())
}

/// Calls an ioctl, retrying on EINTR.
///
/// # Safety
/// `arg` must be null or point to a live value of the type `req` encodes, valid for reads and writes.
pub unsafe fn ioctl<T>(fd: RawFd, req: u32, arg: *mut T) -> io::Result<()> {
    loop {
        // SAFETY: the caller guarantees `arg` matches `req`.
        let r = unsafe { libc::ioctl(fd, req as libc::c_ulong, arg) };
        if r >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// An ioctl that takes no argument.
pub fn ioctl_none(fd: RawFd, req: u32) -> io::Result<()> {
    // SAFETY: a null argument is what the no-argument requests expect.
    unsafe { ioctl::<()>(fd, req, std::ptr::null_mut()) }
}

/// An ioctl whose argument is an int passed by value (`LINK`, `PAUSE`).
pub fn ioctl_int(fd: RawFd, req: u32, value: i32) -> io::Result<()> {
    loop {
        // SAFETY: the request reads an int-sized value from the argument slot.
        let r = unsafe { libc::ioctl(fd, req as libc::c_ulong, value as libc::c_long) };
        if r >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Opens a device node read-write, close-on-exec.
pub fn open_dev(path: &str, nonblock: bool) -> io::Result<RawFd> {
    let c = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let flags = libc::O_RDWR | libc::O_CLOEXEC | if nonblock { libc::O_NONBLOCK } else { 0 };
    // SAFETY: `c` is a valid NUL-terminated string.
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

pub fn close_fd(fd: RawFd) {
    // SAFETY: the caller owns the fd and does not use it afterwards.
    unsafe { libc::close(fd) };
}

fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out pointer.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Host time: `CLOCK_MONOTONIC_RAW` in nanoseconds (the clock of the contract and of the PCM timestamps).
pub fn now_raw_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC_RAW)
}

/// `CLOCK_MONOTONIC` in nanoseconds (the clock timerfd supports).
pub fn now_mono_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioc_encodes_direction_size_type_and_number() {
        // _IOWR('A', 0x11, 608 bytes): dir 3, size 0x260, 'A', 0x11
        assert_eq!(ioc(3, b'A', 0x11, 608), 0xc260_4111);
        assert_eq!(io(b'A', 0x40), 0x4140);
        assert_eq!(ioc(DIR_WRITE, b'A', 0x60, 4), 0x4004_4160);
        assert_eq!(ioc(DIR_READ, b'A', 0x00, 4), 0x8004_4100);
    }

    #[test]
    fn clocks_advance() {
        let a = now_raw_ns();
        let b = now_raw_ns();
        assert!(b >= a);
        assert!(now_mono_ns() > 0);
    }
}
