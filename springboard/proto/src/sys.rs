//! The few libc calls both ends of the app socket need (no libc crate):
//! SOCK_SEQPACKET with SCM_RIGHTS fd passing, memfd (ashmem fallback), mmap, poll.
//! Layouts are the 64-bit Linux/bionic ones (aarch64 and x86_64).

use std::ffi::{c_char, c_int, c_long, c_void};
use std::io;

pub type Fd = c_int;

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    namelen: u32,
    iov: *mut IoVec,
    iovlen: usize,
    control: *mut c_void,
    controllen: usize,
    flags: c_int,
}

#[repr(C)]
struct CmsgHdr {
    len: usize,
    level: c_int,
    kind: c_int,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct TimeSpec {
    sec: i64,
    nsec: i64,
}

extern "C" {
    fn socketpair(domain: c_int, ty: c_int, proto: c_int, sv: *mut c_int) -> c_int;
    fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, arg: c_int) -> c_int;
    fn ftruncate(fd: c_int, len: i64) -> c_int;
    fn lseek(fd: c_int, off: i64, whence: c_int) -> i64;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
    fn syscall(num: c_long, ...) -> c_long;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn ioctl(fd: c_int, req: u64, ...) -> c_int;
    fn clock_gettime(clock: c_int, ts: *mut TimeSpec) -> c_int;
}

const AF_UNIX: c_int = 1;
const SOCK_SEQPACKET: c_int = 5;
const SOCK_CLOEXEC: c_int = 0o2000000;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const MSG_DONTWAIT: c_int = 0x40;
const MSG_NOSIGNAL: c_int = 0x4000;
const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
const F_SETFD: c_int = 2;
const POLLIN: i16 = 1;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MFD_CLOEXEC: u32 = 1;
const MFD_ALLOW_SEALING: u32 = 2;
const F_ADD_SEALS: c_int = 1033;
const F_SEAL_SHRINK: c_int = 2;
const SEEK_END: c_int = 2;
const MSG_TRUNC: c_int = 0x20;
const MSG_CTRUNC: c_int = 0x8;
const O_RDWR: c_int = 2;
const ASHMEM_SET_NAME: u64 = 0x4100_7701;
const ASHMEM_SET_SIZE: u64 = 0x4008_7703;
#[cfg(target_arch = "aarch64")]
const SYS_MEMFD_CREATE: c_long = 279;
#[cfg(target_arch = "x86_64")]
const SYS_MEMFD_CREATE: c_long = 319;

/// CLOCK_MONOTONIC in milliseconds (the clock the shell uses for `t`).
pub fn now_ms() -> f64 {
    let mut ts = TimeSpec { sec: 0, nsec: 0 };
    unsafe { clock_gettime(1, &mut ts) };
    ts.sec as f64 * 1e3 + ts.nsec as f64 / 1e6
}

/// Owns a file descriptor.
#[derive(Debug)]
pub struct Owned(pub Fd);

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { close(self.0) };
    }
}

fn cvt(r: c_int) -> io::Result<c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

/// A connected SOCK_SEQPACKET end.
#[derive(Debug)]
pub struct Channel {
    fd: Owned,
}

/// Both ends of a connected pair, close-on-exec; the app's end is made
/// inheritable by the spawner with `make_inheritable` before exec.
pub fn socketpair_pair() -> io::Result<(Channel, Channel)> {
    let mut sv = [0 as c_int; 2];
    cvt(unsafe { socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sv.as_mut_ptr()) })?;
    Ok((Channel { fd: Owned(sv[0]) }, Channel { fd: Owned(sv[1]) }))
}

/// Clears CLOEXEC so the descriptor is inherited by the exec'd app (call between fork and exec).
pub fn make_inheritable(fd: Fd) {
    unsafe { fcntl(fd, F_SETFD, 0) };
}

/// Grows or shrinks a shared memory file made by `shm_create` (memfd only).
pub fn shm_resize(fd: &Owned, size: usize) -> io::Result<()> {
    cvt(unsafe { ftruncate(fd.0, size as i64) }).map(|_| ())
}

/// Size in bytes of a shared memory file (`lseek` to the end; works for memfd and ashmem).
pub fn shm_len(fd: &Owned) -> io::Result<usize> {
    let n = unsafe { lseek(fd.0, 0, SEEK_END) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// Forbids shrinking the file, so a mapping of its current size can never fault past EOF (memfd only;
/// the creator must have allowed sealing, which `shm_create` does). Idempotent.
pub fn seal_shrink(fd: &Owned) -> io::Result<()> {
    cvt(unsafe { fcntl(fd.0, F_ADD_SEALS, F_SEAL_SHRINK) }).map(|_| ())
}

/// Puts `fd` in non-blocking mode (reads return `WouldBlock` instead of waiting).
pub fn set_nonblocking(fd: Fd) -> io::Result<()> {
    const F_GETFL: c_int = 3;
    const F_SETFL: c_int = 4;
    const O_NONBLOCK: c_int = 0o4000;
    let flags = cvt(unsafe { fcntl(fd, F_GETFL, 0) })?;
    cvt(unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) }).map(|_| ())
}

pub enum Recv {
    /// a packet and the descriptors that came with it
    Packet(Vec<u8>, Vec<Owned>),
    /// nothing queued (non-blocking)
    Empty,
    /// the peer closed
    Closed,
}

const MAX_FDS: usize = 4;
const CMSG_SPACE_FDS: usize = 16 + MAX_FDS * 4;

#[repr(C, align(8))]
struct CtrlBuf([u8; CMSG_SPACE_FDS]);

impl Channel {
    /// Adopts a descriptor (the app side: `BIFROST_SOCK`).
    pub fn from_fd(fd: Fd) -> Self {
        Channel { fd: Owned(fd) }
    }

    pub fn fd(&self) -> Fd {
        self.fd.0
    }

    /// Sends one packet with optional descriptors.
    pub fn send(&self, data: &[u8], fds: &[Fd]) -> io::Result<()> {
        self.send_flags(data, fds, 0)
    }

    /// Like `send` but never waits: a full queue is `ErrorKind::WouldBlock` (the shell's end uses this).
    pub fn try_send(&self, data: &[u8], fds: &[Fd]) -> io::Result<()> {
        self.send_flags(data, fds, MSG_DONTWAIT)
    }

    fn send_flags(&self, data: &[u8], fds: &[Fd], flags: c_int) -> io::Result<()> {
        assert!(fds.len() <= MAX_FDS);
        let mut iov = IoVec { base: data.as_ptr() as *mut c_void, len: data.len() };
        let mut ctrl = CtrlBuf([0; CMSG_SPACE_FDS]);
        let mut msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: std::ptr::null_mut(), controllen: 0, flags: 0 };
        if !fds.is_empty() {
            let len = 16 + fds.len() * 4;
            let h = ctrl.0.as_mut_ptr() as *mut CmsgHdr;
            unsafe {
                *h = CmsgHdr { len, level: SOL_SOCKET, kind: SCM_RIGHTS };
                std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, ctrl.0.as_mut_ptr().add(16), fds.len() * 4);
            }
            msg.control = ctrl.0.as_mut_ptr() as *mut c_void;
            msg.controllen = (len + 7) & !7;
        }
        loop {
            let n = unsafe { sendmsg(self.fd.0, &msg, MSG_NOSIGNAL | flags) };
            if n >= 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }

    /// Receives one packet; `block = false` returns `Empty` when none is queued.
    pub fn recv(&self, block: bool) -> io::Result<Recv> {
        let buf = &mut [0u8; crate::MAX_PACKET];
        let mut iov = IoVec { base: buf.as_mut_ptr() as *mut c_void, len: buf.len() };
        let mut ctrl = CtrlBuf([0; CMSG_SPACE_FDS]);
        let mut msg = MsgHdr {
            name: std::ptr::null_mut(),
            namelen: 0,
            iov: &mut iov,
            iovlen: 1,
            control: ctrl.0.as_mut_ptr() as *mut c_void,
            controllen: CMSG_SPACE_FDS,
            flags: 0,
        };
        let n = loop {
            let n = unsafe { recvmsg(self.fd.0, &mut msg, MSG_CMSG_CLOEXEC | if block { 0 } else { MSG_DONTWAIT }) };
            if n >= 0 {
                break n as usize;
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => return Ok(Recv::Empty),
                _ => return Err(e),
            }
        };
        let mut fds = Vec::new();
        if msg.controllen >= 16 {
            let h = unsafe { &*(ctrl.0.as_ptr() as *const CmsgHdr) };
            if h.level == SOL_SOCKET && h.kind == SCM_RIGHTS && h.len > 16 {
                let count = ((h.len - 16) / 4).min(MAX_FDS);
                for i in 0..count {
                    let mut fd = 0 as c_int;
                    unsafe { std::ptr::copy_nonoverlapping(ctrl.0.as_ptr().add(16 + i * 4), &mut fd as *mut c_int as *mut u8, 4) };
                    fds.push(Owned(fd));
                }
            }
        }
        if msg.flags & (MSG_TRUNC | MSG_CTRUNC) != 0 {
            // an oversized packet or lost descriptors: a protocol error (the fds in `fds` close on drop)
            return Err(io::Error::new(io::ErrorKind::InvalidData, "packet truncated"));
        }
        if n == 0 && fds.is_empty() {
            return Ok(Recv::Closed);
        }
        Ok(Recv::Packet(buf[..n].to_vec(), fds))
    }

    /// Waits until a packet (or EOF) is readable; false on timeout (`None` = forever).
    pub fn wait(&self, timeout_ms: Option<i32>) -> bool {
        let mut p = PollFd { fd: self.fd.0, events: POLLIN, revents: 0 };
        let r = unsafe { poll(&mut p, 1, timeout_ms.unwrap_or(-1)) };
        r > 0
    }

    /// Like `wait`, also watching `extra` (a listening socket). Returns (channel readable, extra readable).
    pub fn wait_with(&self, extra: i32, timeout_ms: Option<i32>) -> (bool, bool) {
        let mut p = [
            PollFd { fd: self.fd.0, events: POLLIN, revents: 0 },
            PollFd { fd: extra, events: POLLIN, revents: 0 },
        ];
        let r = unsafe { poll(p.as_mut_ptr(), 2, timeout_ms.unwrap_or(-1)) };
        if r <= 0 {
            return (false, false);
        }
        (p[0].revents != 0, p[1].revents != 0)
    }

    /// Waits for this channel or any of `extra` (skipped when negative). Returns the channel's readiness and each
    /// extra descriptor's, in order.
    pub fn wait_many(&self, extra: &[i32], timeout_ms: Option<i32>) -> (bool, Vec<bool>) {
        let mut p: Vec<PollFd> = std::iter::once(self.fd.0).chain(extra.iter().copied()).map(|fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
        let r = unsafe { poll(p.as_mut_ptr(), p.len() as u64, timeout_ms.unwrap_or(-1)) };
        if r <= 0 {
            return (false, vec![false; extra.len()]);
        }
        (p[0].revents != 0, p[1..].iter().map(|q| q.revents != 0).collect())
    }
}

/// Shared memory: `memfd_create`, or `/dev/ashmem` where the syscall is refused (Android SELinux).
pub fn shm_create(name: &str, size: usize) -> io::Result<Owned> {
    let cname = format!("{name}\0");
    let fd = unsafe { syscall(SYS_MEMFD_CREATE, cname.as_ptr() as *const c_char, MFD_CLOEXEC | MFD_ALLOW_SEALING) } as c_int;
    if fd >= 0 {
        let fd = Owned(fd);
        cvt(unsafe { ftruncate(fd.0, size as i64) })?;
        return Ok(fd);
    }
    let fd = Owned(cvt(unsafe { open(b"/dev/ashmem\0".as_ptr() as *const c_char, O_RDWR) })?);
    let mut nm = [0u8; 256];
    let n = name.len().min(255);
    nm[..n].copy_from_slice(&name.as_bytes()[..n]);
    cvt(unsafe { ioctl(fd.0, ASHMEM_SET_NAME, nm.as_ptr()) })?;
    cvt(unsafe { ioctl(fd.0, ASHMEM_SET_SIZE, size) })?;
    Ok(fd)
}

/// A shared read/write mapping of `len` bytes.
#[derive(Debug)]
pub struct Map {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping is plain memory owned by this value.
unsafe impl Send for Map {}

impl Map {
    pub fn new(fd: &Owned, len: usize) -> io::Result<Self> {
        let p = unsafe { mmap(std::ptr::null_mut(), len, PROT_READ | PROT_WRITE, MAP_SHARED, fd.0, 0) };
        if p as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Map { ptr: p as *mut u8, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The pixels. The peer writes the same memory, so reads are only
    /// meaningful for the buffer the protocol says this side owns.
    pub fn pixels(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u32, self.len / 4) }
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut u32, self.len / 4) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        unsafe { munmap(self.ptr as *mut c_void, self.len) };
    }
}
