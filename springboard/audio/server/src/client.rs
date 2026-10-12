//! Client connections of the control socket: peer credentials, and one-message-per-packet send and receive with
//! `SCM_RIGHTS` descriptor passing (memfd and eventfd to the client, a pidfd from it). Owner: task 05.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// The most descriptors one message carries.
pub const MAX_FDS: usize = 4;

/// One connected client of the control socket.
pub struct Conn {
    pub fd: OwnedFd,
    pub id: u32,
    pub pid: i32,
    pub uid: u32,
}

impl Conn {
    pub fn new(fd: OwnedFd, id: u32) -> Conn {
        let (pid, uid) = peer_cred(fd.as_raw_fd()).unwrap_or((0, u32::MAX));
        Conn { fd, id, pid, uid }
    }
}

/// `SO_PEERCRED`: (pid, uid) of the process that connected.
pub fn peer_cred(fd: RawFd) -> std::io::Result<(i32, u32)> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid out-parameters of the sizes the call expects.
    let r = unsafe {
        libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len)
    };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok((cred.pid, cred.uid))
    }
}

const CMSG_BUF: usize = 16; // u64 words: room for CMSG_SPACE(MAX_FDS * 4) with alignment to spare

/// Sends one packet with optional descriptors (`MSG_NOSIGNAL`: a closed peer is an error, not SIGPIPE).
pub fn send_msg(fd: RawFd, data: &[u8], fds: &[RawFd]) -> std::io::Result<()> {
    assert!(fds.len() <= MAX_FDS);
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    let mut ctl = [0u64; CMSG_BUF];
    // SAFETY: a zeroed msghdr is a valid empty one.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        let bytes = std::mem::size_of_val(fds);
        // SAFETY: `ctl` is 8-byte aligned and larger than CMSG_SPACE(bytes); the header and data pointers come
        // from the CMSG macros on that buffer, and the descriptors are copied into the data area.
        unsafe {
            msg.msg_control = ctl.as_mut_ptr() as *mut libc::c_void;
            msg.msg_controllen = libc::CMSG_SPACE(bytes as u32) as _;
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(bytes as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, libc::CMSG_DATA(c), bytes);
        }
    }
    // SAFETY: `msg` points at `iov`, `data` and `ctl`, all alive for the call.
    let r = unsafe { libc::sendmsg(fd, &msg, libc::MSG_NOSIGNAL) };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Receives one packet. `Ok((0, _))` is end of stream; a would-block error means nothing is queued. Received
/// descriptors are returned owned (close-on-exec).
pub fn recv_msg(fd: RawFd, buf: &mut [u8]) -> std::io::Result<(usize, Vec<OwnedFd>)> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    let mut ctl = [0u64; CMSG_BUF];
    // SAFETY: a zeroed msghdr is a valid empty one.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = ctl.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = std::mem::size_of_val(&ctl) as _;
    // SAFETY: `msg` points at `iov` (into `buf`) and `ctl`, alive for the call.
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut fds = Vec::new();
    // SAFETY: the kernel filled `ctl` with well-formed control messages of `msg_controllen` bytes; we only read
    // within `cmsg_len` of each header, and take ownership of each descriptor exactly once.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let n = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / 4;
                let data = libc::CMSG_DATA(c) as *const i32;
                for i in 0..n {
                    fds.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(i))));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message truncated"));
    }
    Ok((n as usize, fds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (OwnedFd, OwnedFd) {
        let mut sv = [0i32; 2];
        // SAFETY: `sv` is a two-int out array.
        let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, sv.as_mut_ptr()) };
        assert_eq!(r, 0);
        // SAFETY: fresh descriptors from socketpair.
        unsafe { (OwnedFd::from_raw_fd(sv[0]), OwnedFd::from_raw_fd(sv[1])) }
    }

    #[test]
    fn a_message_and_a_descriptor_arrive_together() {
        let (a, b) = pair();
        // SAFETY: eventfd takes no pointers.
        let efd = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_CLOEXEC)) };
        send_msg(a.as_raw_fd(), b"hello", &[efd.as_raw_fd()]).unwrap();
        let mut buf = [0u8; 64];
        let (n, fds) = recv_msg(b.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
        assert_eq!(fds.len(), 1);
        // Writing through the received descriptor is visible through the original.
        let one = 5u64;
        // SAFETY: valid fd and an 8-byte buffer.
        unsafe { libc::write(fds[0].as_raw_fd(), &one as *const u64 as *const libc::c_void, 8) };
        let mut got = 0u64;
        // SAFETY: valid fd and an 8-byte buffer.
        unsafe { libc::read(efd.as_raw_fd(), &mut got as *mut u64 as *mut libc::c_void, 8) };
        assert_eq!(got, 5);
    }

    #[test]
    fn a_plain_message_has_no_descriptors_and_eof_is_zero() {
        let (a, b) = pair();
        send_msg(a.as_raw_fd(), b"x", &[]).unwrap();
        let mut buf = [0u8; 8];
        let (n, fds) = recv_msg(b.as_raw_fd(), &mut buf).unwrap();
        assert_eq!((n, fds.len()), (1, 0));
        drop(a);
        assert_eq!(recv_msg(b.as_raw_fd(), &mut buf).unwrap().0, 0);
    }

    #[test]
    fn peer_credentials_are_this_process() {
        let (a, _b) = pair();
        let (pid, uid) = peer_cred(a.as_raw_fd()).unwrap();
        assert_eq!(pid, std::process::id() as i32);
        // SAFETY: getuid cannot fail.
        assert_eq!(uid, unsafe { libc::getuid() });
    }

    fn eventfd() -> OwnedFd {
        // SAFETY: eventfd takes no pointers.
        unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_CLOEXEC)) }
    }

    #[test]
    fn the_most_descriptors_arrive_in_order_and_close_on_exec() {
        let (a, b) = pair();
        let sent: Vec<OwnedFd> = (0..MAX_FDS).map(|_| eventfd()).collect();
        let raw: Vec<RawFd> = sent.iter().map(|f| f.as_raw_fd()).collect();
        send_msg(a.as_raw_fd(), b"four", &raw).unwrap();
        let mut buf = [0u8; 16];
        let (n, fds) = recv_msg(b.as_raw_fd(), &mut buf).unwrap();
        assert_eq!((n, fds.len()), (4, MAX_FDS));
        for (i, f) in fds.iter().enumerate() {
            let v = (i + 1) as u64;
            // SAFETY: valid fds and 8-byte buffers.
            unsafe { libc::write(f.as_raw_fd(), &v as *const u64 as *const libc::c_void, 8) };
            let mut got = 0u64;
            unsafe { libc::read(sent[i].as_raw_fd(), &mut got as *mut u64 as *mut libc::c_void, 8) };
            assert_eq!(got, v, "descriptor {i} arrived out of order");
            // SAFETY: F_GETFD on a valid fd.
            let flags = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }
    }

    #[test]
    fn a_packet_larger_than_the_buffer_is_refused() {
        let (a, b) = pair();
        send_msg(a.as_raw_fd(), &[7u8; 32], &[]).unwrap();
        let mut small = [0u8; 8];
        let e = recv_msg(b.as_raw_fd(), &mut small).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn sending_to_a_closed_peer_is_an_error_not_a_signal() {
        let (a, b) = pair();
        drop(b);
        let e = send_msg(a.as_raw_fd(), b"gone", &[]).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EPIPE));
    }

    #[test]
    fn packets_keep_their_boundaries() {
        let (a, b) = pair();
        send_msg(a.as_raw_fd(), b"one", &[]).unwrap();
        send_msg(a.as_raw_fd(), b"three", &[]).unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(recv_msg(b.as_raw_fd(), &mut buf).unwrap().0, 3);
        assert_eq!(recv_msg(b.as_raw_fd(), &mut buf).unwrap().0, 5);
    }
}
