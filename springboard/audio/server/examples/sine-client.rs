//! A raw test client of `bifrost-audiod` (the HAL client library, task 06, replaces this for real apps): connects to
//! the control socket, adds a stereo output IO procedure, and plays a sine for N seconds from its own real-time
//! thread, following the shared-memory protocol of docs/audio-architecture.md ("Client IO").
//!
//! Run two at once on Midget to hear (or measure) the mix:
//!   bifrost-audiod &                                  # or the systemd unit
//!   cargo run --example sine-client -- 440 60 256     # frequency Hz, seconds, buffer frames
//!   cargo run --example sine-client -- 660 60 256
//! Optional fourth argument: the device object id (default 1000, the first device).

use bifrost_audio_shm::{self as shm, Activation, Region};
use bifrost_audio_types::control::{
    decode_reply, encode_request, IoProcRequest, Reply, Request, CONTROL_SOCKET, MAX_LEN, PROTOCOL_VERSION,
};
use bifrost_audio_types::ObjectId;
use bifrost_audiod::client::{recv_msg, send_msg};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn connect(path: &str) -> std::io::Result<OwnedFd> {
    // SAFETY: socket takes no pointers; the descriptor is checked and owned.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fresh descriptor.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: a zeroed sockaddr_un is valid; the path is copied within sun_path's bounds.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (d, s) in addr.sun_path.iter_mut().zip(path.as_bytes()) {
        *d = *s as libc::c_char;
    }
    // SAFETY: `addr` is a valid sockaddr_un of the size passed.
    let r = unsafe { libc::connect(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_un>() as u32) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(owned)
}

fn call(sock: &OwnedFd, req: &Request) -> (Reply, Vec<OwnedFd>) {
    send_msg(sock.as_raw_fd(), &encode_request(req), &[]).expect("send");
    let mut buf = vec![0u8; MAX_LEN];
    loop {
        let (n, fds) = recv_msg(sock.as_raw_fd(), &mut buf).expect("recv");
        let reply = decode_reply(&buf[..n]).expect("decode");
        // Skip asynchronous notices that arrive between request and reply.
        if !matches!(reply, Reply::Overload { .. } | Reply::Changed(_)) {
            return (reply, fds);
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let freq: f32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(440.0);
    let secs: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let buffer: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(256);
    let device: u32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000);

    let sock = connect(CONTROL_SOCKET).expect("connect to bifrost-audiod");
    let (r, _) = call(&sock, &Request::Hello { version: PROTOCOL_VERSION, client: "sine-client".into(), policy: false });
    assert!(matches!(r, Reply::Welcome { .. }), "hello: {r:?}");
    let (r, mut fds) = call(
        &sock,
        &Request::AddIoProc(IoProcRequest { device: ObjectId(device), input_channels: 0, output_channels: 2, buffer_frames: buffer }),
    );
    let Reply::IoProcAdded { id, layout } = r else { panic!("add io proc: {r:?}") };
    let memfd = fds.remove(0);
    let region = Region::map(memfd, layout.total as usize).expect("map");
    let act: &Activation = region.get(layout.activation);
    let done: &Activation = region.get(layout.done);
    let out = region.ring_trusted(layout.output_ring, 2, layout.ring_frames).expect("ring");

    // Ask the server for real-time priority on this thread (no CAP_SYS_NICE needed here).
    // SAFETY: gettid takes no arguments.
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as i32;
    let (r, _) = call(&sock, &Request::PromoteThread { tid });
    eprintln!("promote: {r:?}");
    let (r, _) = call(&sock, &Request::StartIoProc { id });
    assert_eq!(r, Reply::Done);

    let start = Instant::now();
    let (mut phase, mut cycles, mut woke) = (0.0f32, 0u64, 0u64);
    let mut seen = act.wake.load(Ordering::Acquire);
    let mut buf = vec![0.0f32; 4096];
    while start.elapsed() < Duration::from_secs(secs) {
        shm::futex::wait(&act.wake, seen, 100_000_000);
        let w = act.wake.load(Ordering::Acquire);
        if w == seen {
            continue;
        }
        seen = w;
        woke += 1;
        // The server stores this cycle's quantum and rate before it wakes us.
        let q = act.quantum.load(Ordering::Acquire) as usize;
        let rate = act.rate.load(Ordering::Acquire).max(1) as f32;
        let q = q.min(buf.len());
        for s in &mut buf[..q] {
            *s = 0.2 * (phase * std::f32::consts::TAU).sin();
            phase = (phase + freq / rate).fract();
        }
        out.write(&[&buf[..q], &buf[..q]], q as u32);
        shm::finish(act, &[done], 0);
        cycles += 1;
    }
    let (r, _) = call(&sock, &Request::StopIoProc { id });
    let (_, _) = call(&sock, &Request::RemoveIoProc { id });
    eprintln!("sine-client: {cycles} cycles, {woke} wakes in {secs} s, stop: {r:?}");
}
