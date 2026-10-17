//! Mounts a read-only FUSE volume through the `fusermount3` helper and serves its requests (`fuse::decode` and
//! `fs_module::Dispatcher` do the protocol work).
//!
//! The helper protocol is libfuse's: make a Unix socket pair, give the helper one end in the environment variable
//! `_FUSE_COMMFD`, run `fusermount3 -o OPTIONS -- MOUNTPOINT`, and read the open `/dev/fuse` descriptor back from the
//! helper with `SCM_RIGHTS`. The helper opens `/dev/fuse` and performs the mount, so the caller needs no privilege
//! beyond what the helper allows unprivileged users. Mounting directly with `mount(2)` needs `CAP_SYS_ADMIN` and is not
//! built here.
//!
//! Limits: the helper must be installed on the PATH; requests are served on the calling thread; there is no shell mount
//! service, grant check or notification of the file panel yet (see docs/bifrost-ui.md, "Unary file system module").

use std::ffi::{c_int, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fs_module::{Dispatcher, UnaryFileSystem};

const AF_UNIX: c_int = 1;
const SOCK_STREAM: c_int = 1;
const SOCK_CLOEXEC: c_int = 0o2000000;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
const F_SETFD: c_int = 2;
const EINTR: i32 = 4;
const ENOENT: i32 = 2;
const ENODEV: i32 = 19;

/// Room for one request: a full write plus the header, as the kernel may send.
const REQUEST_BUFFER: usize = crate::fuse::MAX_WRITE as usize + 4096;

#[repr(C)]
struct IoVec {
    iov_base: *mut c_void,
    iov_len: usize,
}

/// `struct msghdr` on 64-bit Linux glibc.
#[repr(C)]
struct MsgHdr {
    msg_name: *mut c_void,
    msg_namelen: u32,
    msg_iov: *mut IoVec,
    msg_iovlen: usize,
    msg_control: *mut c_void,
    msg_controllen: usize,
    msg_flags: c_int,
}

/// `struct cmsghdr` on 64-bit Linux; the descriptor follows it, aligned to 8 bytes.
#[repr(C)]
struct CmsgHdr {
    cmsg_len: usize,
    cmsg_level: c_int,
    cmsg_type: c_int,
}

extern "C" {
    fn socketpair(domain: c_int, ty: c_int, protocol: c_int, sv: *mut c_int) -> c_int;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
}

/// A mounted FUSE session: the `/dev/fuse` descriptor the helper opened, and where it is mounted.
pub struct Mount {
    device: File,
    mountpoint: PathBuf,
}

impl Mount {
    /// Mounts a read-only volume at `mountpoint`. `fsname` is shown in the mount table and must not contain a comma.
    pub fn mount(mountpoint: &Path, fsname: &str) -> io::Result<Mount> {
        if fsname.is_empty() || fsname.contains(',') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "fsname must be non-empty and contain no comma"));
        }
        let (parent_end, child_end) = socket_pair()?;
        // The helper inherits its end across exec, so the close-on-exec flag is cleared on that end only.
        // SAFETY: `child_end` is a descriptor this function owns, and F_SETFD takes an int argument.
        if unsafe { fcntl(child_end.as_raw_fd(), F_SETFD, 0 as c_int) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let options = format!("ro,default_permissions,fsname={fsname},subtype=bifrost");
        let mut child = Command::new("fusermount3")
            .arg("-o")
            .arg(options)
            .arg("--")
            .arg(mountpoint)
            .env("_FUSE_COMMFD", child_end.as_raw_fd().to_string())
            .spawn()?;
        // Dropping our copy of the helper's end lets the helper's exit close the stream.
        drop(child_end);
        let received = receive_fd(parent_end.as_raw_fd());
        let status = child.wait()?;
        if !status.success() {
            return Err(io::Error::other(format!("fusermount3 exited with {status}")));
        }
        Ok(Mount {
            device: File::from(received?),
            mountpoint: mountpoint.to_path_buf(),
        })
    }

    /// Serves requests for `fs` on the calling thread until the session ends: the kernel closes it on unmount, which
    /// shows up as ENODEV on read.
    pub fn serve<F: UnaryFileSystem>(&mut self, fs: F) -> io::Result<()> {
        let dispatcher = Dispatcher::new(fs);
        let mut buf = vec![0u8; REQUEST_BUFFER];
        loop {
            let n = match self.device.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                // Interrupted or withdrawn request: read the next one.
                Err(e) if matches!(e.raw_os_error(), Some(EINTR) | Some(ENOENT)) => continue,
                Err(e) if e.raw_os_error() == Some(ENODEV) => return Ok(()),
                Err(e) => return Err(e),
            };
            if let Some(reply) = dispatcher.handle(&buf[..n]) {
                match self.device.write_all(&reply) {
                    Ok(()) => {}
                    // The kernel withdrew the request while we worked on it; the reply is not wanted.
                    Err(e) if e.raw_os_error() == Some(ENOENT) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// Unmounts through the helper (`fusermount3 -u`). The session ends once the device is closed.
    pub fn unmount(self) -> io::Result<()> {
        let status = Command::new("fusermount3")
            .arg("-u")
            .arg("--")
            .arg(&self.mountpoint)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("fusermount3 -u exited with {status}")))
        }
    }

    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }
}

fn socket_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds: [c_int; 2] = [-1, -1];
    // SAFETY: `fds` has room for the two descriptors socketpair writes.
    if unsafe { socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, fds.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socketpair succeeded, so both descriptors are new and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Reads the one descriptor the helper sends with `SCM_RIGHTS`.
fn receive_fd(socket: RawFd) -> io::Result<OwnedFd> {
    let mut data = [0u8; 1];
    let mut iov = IoVec {
        iov_base: data.as_mut_ptr() as *mut c_void,
        iov_len: data.len(),
    };
    // Room for one control message carrying one descriptor: header (16) plus an int, padded to 8, is 24 bytes.
    let mut control = [0u64; 4];
    let mut msg = MsgHdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &mut iov as *mut IoVec,
        msg_iovlen: 1,
        msg_control: control.as_mut_ptr() as *mut c_void,
        msg_controllen: std::mem::size_of_val(&control),
        msg_flags: 0,
    };
    let n = loop {
        // SAFETY: `msg` points at buffers that live for this call, with the sizes they were given above.
        let n = unsafe { recvmsg(socket, &mut msg, MSG_CMSG_CLOEXEC) };
        if n < 0 && io::Error::last_os_error().raw_os_error() == Some(EINTR) {
            continue;
        }
        break n;
    };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let header_len = std::mem::size_of::<CmsgHdr>();
    if msg.msg_controllen < header_len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "fusermount3 sent no descriptor"));
    }
    // SAFETY: `control` holds at least `header_len` bytes written by the kernel (checked above); the read is unaligned.
    let header: CmsgHdr = unsafe { std::ptr::read_unaligned(control.as_ptr() as *const CmsgHdr) };
    if header.cmsg_level == SOL_SOCKET && header.cmsg_type == SCM_RIGHTS && header.cmsg_len >= header_len + 4 {
        // SAFETY: the control message carries at least one int after its header, as checked by the length above.
        let fd: c_int = unsafe { std::ptr::read_unaligned((control.as_ptr() as *const u8).add(header_len) as *const c_int) };
        // SAFETY: the kernel installed this descriptor for us with MSG_CMSG_CLOEXEC, and nothing else owns it.
        return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "fusermount3 sent no descriptor"))
}
