//! The control-socket connection: SEQPACKET framing with SCM_RIGHTS, a reader thread that splits replies from
//! notifications, and synchronous requests. Not real time.

use bifrost_audio_types::control::{decode_reply, encode_request, Reply, Request, MAX_LEN};
use bifrost_audio_types::{AudioError, ObjectId, PropertyAddress};
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a request waits for its reply.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// The most fds one packet may carry.
const MAX_FDS: usize = 4;

pub(crate) type ListenerFn = dyn Fn(ObjectId, PropertyAddress) + Send + Sync;
pub(crate) type Msg = (Reply, Vec<OwnedFd>);

/// State the reader thread shares with the callers.
pub(crate) struct Shared {
    pub listeners: Mutex<HashMap<(ObjectId, PropertyAddress), Vec<(u64, Arc<ListenerFn>)>>>,
    pub overloads: Mutex<HashMap<u32, u32>>,
    pub dead: AtomicBool,
}

pub(crate) struct Conn {
    pub sock: Arc<OwnedFd>,
    rx: Mutex<Receiver<Msg>>,
    pub shared: Arc<Shared>,
}

/// Sends one packet with optional fds.
pub(crate) fn send_packet(fd: RawFd, data: &[u8], fds: &[RawFd]) -> io::Result<()> {
    if fds.len() > MAX_FDS {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE((fds.len() * 4) as u32) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8).max(1)];
    // SAFETY: msghdr is plain data; all-zero is a valid empty value.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = space as _;
        // SAFETY: msg_control points at `space` zeroed, 8-aligned bytes, enough for one SCM_RIGHTS header and
        // `fds.len()` ints, so the first header and its data area are in bounds.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN((fds.len() * 4) as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, libc::CMSG_DATA(c), fds.len() * 4);
        }
    }
    loop {
        // SAFETY: `msg` and the buffers it points at outlive the call.
        let n = unsafe { libc::sendmsg(fd, &msg, libc::MSG_NOSIGNAL) };
        if n >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// The reply tags that are notifications, not answers to a request (`control::encode_reply`: 7 Changed, 8 Overload).
const NOTIFICATION_TAGS: [u8; 2] = [7, 8];

/// What the reader does with a packet that did not fit its buffer (or whose fds did not fit): SEQPACKET has already
/// discarded the rest of it, so it is dropped. Its first byte (still in the buffer) is the reply tag: a reply answers
/// the waiting request with `Protocol`; a notification is only logged. `None` means nobody is waiting on it.
pub(crate) fn oversized_answer(first_byte: Option<u8>) -> Option<Msg> {
    match first_byte {
        Some(t) if NOTIFICATION_TAGS.contains(&t) => None,
        _ => Some((Reply::Error(AudioError::Protocol), Vec::new())),
    }
}

/// Receives one packet and the fds that came with it. Ok(None) at end of stream. A packet longer than `buf` (or
/// with more fds than fit) is `InvalidData`: its first `buf.len()` bytes are in `buf`, the kernel dropped the rest,
/// and any fds that did arrive are closed; the socket stays usable for the next packet.
pub(crate) fn recv_packet(fd: RawFd, buf: &mut [u8]) -> io::Result<Option<(usize, Vec<OwnedFd>)>> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    // SAFETY: pure size computation.
    let space = unsafe { libc::CMSG_SPACE((MAX_FDS * 4) as u32) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8)];
    // SAFETY: all-zero msghdr is valid.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = space as _;
    let n = loop {
        // SAFETY: `msg` and its buffers outlive the call.
        let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut fds = Vec::new();
    // SAFETY: the kernel filled the control buffer; the CMSG macros walk it within `msg_controllen`, and every
    // fd read here is owned by this process from now on (wrapped at once so it closes on any early return).
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let len = ((*c).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                for i in 0..len / 4 {
                    let raw = std::ptr::read_unaligned(libc::CMSG_DATA(c).add(i * 4) as *const i32);
                    fds.push(OwnedFd::from_raw_fd(raw));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    if n == 0 && fds.is_empty() {
        return Ok(None);
    }
    Ok(Some((n, fds)))
}

impl Conn {
    /// Wraps a connected SEQPACKET socket and starts the reader (notification) thread.
    pub fn new(sock: OwnedFd) -> Arc<Conn> {
        let sock = Arc::new(sock);
        let shared = Arc::new(Shared { listeners: Mutex::new(HashMap::new()), overloads: Mutex::new(HashMap::new()), dead: AtomicBool::new(false) });
        let (tx, rx) = channel::<Msg>();
        let (s2, sh2) = (sock.clone(), shared.clone());
        std::thread::Builder::new()
            .name("audio-notify".into())
            .spawn(move || {
                let mut buf = vec![0u8; MAX_LEN];
                loop {
                    let (n, fds) = match recv_packet(s2.as_raw_fd(), &mut buf) {
                        Ok(Some(x)) => x,
                        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                            // too big for MAX_LEN: already truncated and dropped by the kernel; keep reading
                            eprintln!("[is.olibuijr.audio/hal] dropped an oversized packet (tag {})", buf[0]);
                            if let Some(m) = oversized_answer(buf.first().copied()) {
                                let _ = tx.send(m);
                            }
                            continue;
                        }
                        Ok(None) | Err(_) => break,
                    };
                    let reply = match decode_reply(&buf[..n]) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[is.olibuijr.audio/hal] undecodable reply: {e:?}");
                            continue;
                        }
                    };
                    match reply {
                        Reply::Changed(list) => {
                            for key in list {
                                let fs: Vec<Arc<ListenerFn>> = sh2.listeners.lock().unwrap().get(&key).map(|v| v.iter().map(|(_, f)| f.clone()).collect()).unwrap_or_default();
                                for f in fs {
                                    f(key.0, key.1);
                                }
                            }
                        }
                        Reply::Overload { id, count } => {
                            *sh2.overloads.lock().unwrap().entry(id).or_insert(0) += count;
                        }
                        other => {
                            let _ = tx.send((other, fds));
                        }
                    }
                }
                sh2.dead.store(true, Ordering::Release);
                // dropping `tx` wakes a waiting request with a disconnect
            })
            .expect("spawn audio-notify");
        Arc::new(Conn { sock, rx: Mutex::new(rx), shared })
    }

    /// One request, one reply (and the fds that came with it). `Reply::Error` becomes `Err`.
    pub fn request(&self, req: &Request, fds: &[RawFd]) -> Result<Msg, AudioError> {
        let rx = self.rx.lock().unwrap();
        if self.shared.dead.load(Ordering::Acquire) {
            return Err(AudioError::NoServer);
        }
        while rx.try_recv().is_ok() {}
        send_packet(self.sock.as_raw_fd(), &encode_request(req), fds).map_err(|_| AudioError::NoServer)?;
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok((Reply::Error(e), _)) => Err(e),
            Ok(m) => Ok(m),
            Err(RecvTimeoutError::Timeout) => Err(AudioError::Io),
            Err(RecvTimeoutError::Disconnected) => Err(AudioError::NoServer),
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: shutdown on a valid socket fd ends the reader thread's recvmsg.
        unsafe { libc::shutdown(self.sock.as_raw_fd(), libc::SHUT_RDWR) };
    }
}

#[cfg(test)]
pub(crate) fn socket_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0i32; 2];
    // SAFETY: fds has room for two ints.
    let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) };
    assert_eq!(r, 0);
    // SAFETY: socketpair returned two fresh fds.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_keeps_its_bytes_and_boundaries() {
        let (a, b) = socket_pair();
        send_packet(a.as_raw_fd(), &[1, 2, 3], &[]).unwrap();
        send_packet(a.as_raw_fd(), &[4, 5], &[]).unwrap();
        let mut buf = [0u8; 64];
        let (n, fds) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((&buf[..n], fds.len()), (&[1u8, 2, 3][..], 0));
        let (n, _) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], &[4, 5]);
    }

    #[test]
    fn two_fds_arrive_with_the_packet_and_work() {
        let (a, b) = socket_pair();
        let (x, y) = socket_pair();
        send_packet(a.as_raw_fd(), &[9], &[x.as_raw_fd(), y.as_raw_fd()]).unwrap();
        let mut buf = [0u8; 8];
        let (n, fds) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((n, fds.len()), (1, 2));
        // fds[0] is a dup of x; what it sends, y (the other end) reads.
        send_packet(fds[0].as_raw_fd(), &[7], &[]).unwrap();
        let (n, _) = recv_packet(y.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((n, buf[0]), (1, 7));
    }

    /// A scripted peer: for each request it receives it sends the next batch of replies.
    fn script(sock: OwnedFd, batches: Vec<Vec<Reply>>) -> std::thread::JoinHandle<()> {
        use bifrost_audio_types::control::encode_reply;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; MAX_LEN];
            for batch in batches {
                match recv_packet(sock.as_raw_fd(), &mut buf) {
                    Ok(Some(_)) => {}
                    _ => return,
                }
                for r in batch {
                    if send_packet(sock.as_raw_fd(), &encode_reply(&r), &[]).is_err() {
                        return;
                    }
                }
            }
        })
    }

    fn ping() -> Request {
        Request::Unlisten { object: ObjectId(1), address: PropertyAddress::global(bifrost_audio_types::props::NAME) }
    }

    #[test]
    fn an_error_reply_is_an_err() {
        let (a, b) = socket_pair();
        let _srv = script(b, vec![vec![Reply::Error(AudioError::NotSettable)], vec![Reply::Done]]);
        let c = Conn::new(a);
        assert_eq!(c.request(&ping(), &[]).err(), Some(AudioError::NotSettable));
        // the connection stays usable after an error reply
        assert!(matches!(c.request(&ping(), &[]), Ok((Reply::Done, _))));
    }

    #[test]
    fn a_reply_larger_than_a_small_buffer_arrives_whole() {
        let (a, b) = socket_pair();
        let ids: Vec<ObjectId> = (0..8000).map(ObjectId).collect();
        let _srv = script(b, vec![vec![Reply::Objects(ids.clone())]]);
        let c = Conn::new(a);
        match c.request(&ping(), &[]) {
            Ok((Reply::Objects(got), _)) => assert_eq!(got, ids),
            _ => panic!("the large reply did not arrive as Objects"),
        }
    }

    #[test]
    fn notifications_interleaved_with_a_reply_do_not_disturb_it() {
        let (a, b) = socket_pair();
        let key = (ObjectId(4), PropertyAddress::global(bifrost_audio_types::props::NAME));
        let _srv = script(b, vec![vec![Reply::Changed(vec![key]), Reply::Done, Reply::Changed(vec![key])]]);
        let c = Conn::new(a);
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let f: Arc<ListenerFn> = Arc::new(move |o, _| {
            let _ = tx.lock().unwrap().send(o);
        });
        c.shared.listeners.lock().unwrap().insert(key, vec![(1, f)]);
        assert!(matches!(c.request(&ping(), &[]), Ok((Reply::Done, _))));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), ObjectId(4));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), ObjectId(4));
    }

    #[test]
    fn a_new_connection_works_after_the_server_restarts() {
        let (a, b) = socket_pair();
        let srv = script(b, vec![vec![Reply::Done]]);
        let c = Conn::new(a);
        assert!(c.request(&ping(), &[]).is_ok());
        srv.join().unwrap(); // the old server is gone: its end of the socket is closed
        assert_eq!(c.request(&ping(), &[]).err(), Some(AudioError::NoServer));
        assert!(c.shared.dead.load(Ordering::Acquire) || c.request(&ping(), &[]).is_err());
        // the "restarted" server: a fresh socket and a fresh Conn
        let (a2, b2) = socket_pair();
        let _srv2 = script(b2, vec![vec![Reply::Done]]);
        let c2 = Conn::new(a2);
        assert!(matches!(c2.request(&ping(), &[]), Ok((Reply::Done, _))));
    }

    /// A peer that answers each request with the next list of raw packets.
    fn script_raw(sock: OwnedFd, batches: Vec<Vec<Vec<u8>>>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut buf = vec![0u8; MAX_LEN];
            for batch in batches {
                match recv_packet(sock.as_raw_fd(), &mut buf) {
                    Ok(Some(_)) => {}
                    _ => return,
                }
                for p in batch {
                    if send_packet(sock.as_raw_fd(), &p, &[]).is_err() {
                        return;
                    }
                }
            }
        })
    }

    /// `len` bytes that start with reply tag `tag`.
    fn oversized(tag: u8, len: usize) -> Vec<u8> {
        let mut p = vec![0u8; len];
        p[0] = tag;
        p
    }

    #[test]
    fn oversized_answer_by_tag() {
        // 4 = Objects (a reply): the waiting request gets Protocol
        assert!(matches!(oversized_answer(Some(4)), Some((Reply::Error(AudioError::Protocol), ref f)) if f.is_empty()));
        // 7 = Changed and 8 = Overload are notifications: nobody waits on them
        assert!(oversized_answer(Some(7)).is_none());
        assert!(oversized_answer(Some(8)).is_none());
        // nothing read: answer the request rather than leave it to time out
        assert!(oversized_answer(None).is_some());
    }

    #[test]
    fn a_truncated_packet_is_invalid_data_and_the_next_one_reads_whole() {
        let (a, b) = socket_pair();
        send_packet(a.as_raw_fd(), &[4, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[]).unwrap();
        send_packet(a.as_raw_fd(), &[3], &[]).unwrap();
        let mut buf = [0u8; 4];
        let e = recv_packet(b.as_raw_fd(), &mut buf).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        // the head of the dropped packet is in the buffer
        assert_eq!(buf, [4, 1, 2, 3]);
        // the rest was discarded with it: the next read is the next packet
        let (n, _) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], &[3]);
    }

    #[test]
    fn an_oversized_reply_is_a_protocol_error_and_the_reader_lives_on() {
        use bifrost_audio_types::control::encode_reply;
        let (a, b) = socket_pair();
        let done = encode_reply(&Reply::Done);
        let _srv = script_raw(
            b,
            vec![
                // request 1: a reply (Objects) one byte over the limit
                vec![oversized(4, MAX_LEN + 1)],
                // request 2: an oversized notification (Changed), then the real reply
                vec![oversized(7, MAX_LEN + 100), done.clone()],
                // request 3: a normal reply
                vec![done],
            ],
        );
        let c = Conn::new(a);
        assert_eq!(c.request(&ping(), &[]).err(), Some(AudioError::Protocol));
        assert!(!c.shared.dead.load(Ordering::Acquire));
        assert!(matches!(c.request(&ping(), &[]), Ok((Reply::Done, _))));
        assert!(matches!(c.request(&ping(), &[]), Ok((Reply::Done, _))));
        assert!(!c.shared.dead.load(Ordering::Acquire));
    }

    #[test]
    fn end_of_stream_is_none() {
        let (a, b) = socket_pair();
        drop(a);
        let mut buf = [0u8; 8];
        assert!(recv_packet(b.as_raw_fd(), &mut buf).unwrap().is_none());
    }

    #[test]
    fn more_than_four_fds_is_refused() {
        let (a, _b) = socket_pair();
        let fd = a.as_raw_fd();
        assert!(send_packet(fd, &[0], &[fd; 5]).is_err());
    }
}
