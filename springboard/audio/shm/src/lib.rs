//! `bifrost-audio-shm`: the shared-memory ABI between `bifrost-audiod` and its clients (docs/audio-architecture.md,
//! "IO cycle", "Client IO" and the PipeWire scheduling ideas): activation records, the zero-timestamp seqlock, the
//! single-producer single-consumer rings, the futex and eventfd wakers, and the graph trigger.
//!
//! Every shared struct is `#[repr(C)]` with its size and field offsets pinned by tests. This and the drivers are the
//! only crates with `unsafe`; each block carries a SAFETY comment. Owner: task 03.
//!
//! Real-time rules: `finish`, `reset`, `Activation::wait_triggered`, `ZeroStamp::{publish, read}` and `Ring::{read,
//! write}` never allocate or lock; the only syscall is the futex.

use core::cell::UnsafeCell;
use core::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

/// A node's activation record, one 64-byte cache line per node.
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct Activation {
    /// `STATUS_*`.
    pub status: AtomicU32,
    /// Inputs still to finish this cycle; the node runs when it reaches 0.
    pub pending: AtomicU32,
    /// How many inputs the node has (`pending` is reset to this at cycle start).
    pub required: u32,
    pub xruns: AtomicU32,
    /// When the node was signalled, woke and finished this cycle (host ns), for the profiler.
    pub signal_ns: AtomicU64,
    pub awake_ns: AtomicU64,
    pub finish_ns: AtomicU64,
    /// The graph's frames per cycle and rate for this cycle.
    pub quantum: AtomicU32,
    pub rate: AtomicU32,
    /// The futex word the node sleeps on (incremented to wake it).
    pub wake: AtomicU32,
    pub _pad: u32,
}

pub const STATUS_IDLE: u32 = 0;
pub const STATUS_TRIGGERED: u32 = 1;
pub const STATUS_AWAKE: u32 = 2;
pub const STATUS_FINISHED: u32 = 3;

/// The device's zero timestamp, published by the IO thread and read by clients without a syscall.
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct ZeroStamp {
    /// Odd while a write is in progress (seqlock).
    pub seq: AtomicU32,
    pub seed: AtomicU32,
    pub sample_time: AtomicU64,
    pub host_ns: AtomicU64,
    /// `f64::to_bits` of the rate scalar.
    pub rate_scalar: AtomicU64,
}

/// A consistent copy of a zero timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ZeroStampValue {
    pub sample_time: u64,
    pub host_ns: u64,
    pub rate_scalar: f64,
    pub seed: u32,
}

/// How often `ZeroStamp::read` retries a torn read before it gives up.
const ZEROSTAMP_RETRIES: u32 = 1024;

impl ZeroStamp {
    /// Publishes a new value (single writer: the device's IO thread).
    pub fn publish(&self, v: ZeroStampValue) {
        let s = self.seq.load(Ordering::Relaxed);
        // Odd: a write is in progress. The release fence keeps the field stores after this store.
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.seed.store(v.seed, Ordering::Relaxed);
        self.sample_time.store(v.sample_time, Ordering::Relaxed);
        self.host_ns.store(v.host_ns, Ordering::Relaxed);
        self.rate_scalar.store(v.rate_scalar.to_bits(), Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// Reads a consistent value, retrying on a torn read (wait-free for the reader in the absence of a writer).
    /// The retries are bounded: if the writer died mid-update (seq stuck odd) the last values seen are returned
    /// instead of spinning forever; the caller extrapolates from them.
    pub fn read(&self) -> ZeroStampValue {
        self.read_checked().0
    }

    /// Like [`read`](Self::read), and also says whether the value is consistent (false: the retries ran out, the
    /// value is the last, possibly torn, read).
    pub fn read_checked(&self) -> (ZeroStampValue, bool) {
        let mut last = ZeroStampValue::default();
        for _ in 0..ZEROSTAMP_RETRIES {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 0 {
                let v = ZeroStampValue {
                    seed: self.seed.load(Ordering::Relaxed),
                    sample_time: self.sample_time.load(Ordering::Relaxed),
                    host_ns: self.host_ns.load(Ordering::Relaxed),
                    rate_scalar: f64::from_bits(self.rate_scalar.load(Ordering::Relaxed)),
                };
                fence(Ordering::Acquire);
                if self.seq.load(Ordering::Relaxed) == s1 {
                    return (v, true);
                }
                last = v;
            }
            core::hint::spin_loop();
        }
        (last, false)
    }
}

/// The header of a single-producer single-consumer ring of f32 frames (non-interleaved, `channels` planes of
/// `frames` samples each, following the header in the memfd).
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct RingHeader {
    /// Frames written so far (producer), wrapping at u64.
    pub write: AtomicU64,
    /// Frames read so far (consumer).
    pub read: AtomicU64,
    pub frames: u32,
    pub channels: u32,
}

/// Bytes of a ring (header plus planes), rounded up to 64.
pub fn ring_bytes(channels: u32, frames: u32) -> u32 {
    let planes = channels as u64 * frames as u64 * 4;
    ((64 + planes + 63) & !63) as u32
}

/// A ring view over a header and its planes in a mapping. Producer and consumer each hold one.
pub struct Ring<'a> {
    pub header: &'a RingHeader,
    pub planes: &'a [UnsafeCell<f32>],
    /// Frames per plane and plane count, copied at construction from values the owner trusts. The header's own
    /// `frames`/`channels` live in memory the peer can rewrite and are never read after the view is built.
    cap: u32,
    channels: u32,
}

impl<'a> Ring<'a> {
    /// A view of `channels` planes of `frames` frames. Fails when `planes` is smaller than `channels * frames`.
    pub fn new(header: &'a RingHeader, planes: &'a [UnsafeCell<f32>], channels: u32, frames: u32) -> Option<Ring<'a>> {
        let cells = (channels as usize).checked_mul(frames as usize)?;
        if cells > planes.len() {
            return None;
        }
        Some(Ring { header, planes, cap: frames, channels })
    }

    fn cap(&self) -> u32 {
        self.cap
    }

    /// Frames available to read. A corrupt header from the peer is clamped to the capacity.
    pub fn readable(&self) -> u32 {
        let w = self.header.write.load(Ordering::Acquire);
        let r = self.header.read.load(Ordering::Acquire);
        w.wrapping_sub(r).min(self.cap() as u64) as u32
    }

    /// Frames of free space to write.
    pub fn writable(&self) -> u32 {
        self.cap() - self.readable()
    }

    /// Writes up to `frames` frames from per-channel slices; returns the frames written.
    pub fn write(&self, src: &[&[f32]], frames: u32) -> u32 {
        let cap = self.cap();
        let ch = (self.channels as usize).min(src.len());
        let mut n = frames.min(self.writable());
        for s in &src[..ch] {
            n = n.min(s.len() as u32);
        }
        if n == 0 || cap == 0 {
            return 0;
        }
        let w = self.header.write.load(Ordering::Relaxed);
        let start = (w % cap as u64) as usize;
        for (c, s) in src[..ch].iter().enumerate() {
            self.copy_in(c, start, &s[..n as usize]);
        }
        self.header.write.store(w.wrapping_add(n as u64), Ordering::Release);
        n
    }

    /// Reads up to `frames` frames into per-channel slices; returns the frames read.
    pub fn read(&self, dst: &mut [&mut [f32]], frames: u32) -> u32 {
        let cap = self.cap();
        let ch = (self.channels as usize).min(dst.len());
        let mut n = frames.min(self.readable());
        for d in &dst[..ch] {
            n = n.min(d.len() as u32);
        }
        if n == 0 || cap == 0 {
            return 0;
        }
        let r = self.header.read.load(Ordering::Relaxed);
        let start = (r % cap as u64) as usize;
        for (c, d) in dst[..ch].iter_mut().enumerate() {
            self.copy_out(c, start, &mut d[..n as usize]);
        }
        self.header.read.store(r.wrapping_add(n as u64), Ordering::Release);
        n
    }

    fn plane_ptr(&self, c: usize) -> *mut f32 {
        let cap = self.cap() as usize;
        assert!(c < self.channels as usize && (c + 1) * cap <= self.planes.len(), "ring plane out of bounds");
        // SAFETY: the plane slice holds at least channels * cap cells (checked in `Ring::new`) and c < channels
        // (asserted above), so the plane starts and ends inside the slice.
        unsafe { UnsafeCell::raw_get(self.planes.as_ptr().add(c * cap)) }
    }

    fn copy_in(&self, c: usize, start: usize, s: &[f32]) {
        let cap = self.cap() as usize;
        assert!(start <= cap && s.len() <= cap, "ring write out of bounds");
        let first = s.len().min(cap - start);
        let p = self.plane_ptr(c);
        // SAFETY: SPSC: only the producer writes the free region [write, write + n), which the consumer does not
        // touch until the release store of `write`; both copies stay inside the plane (start + first <= cap).
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), p.add(start), first);
            core::ptr::copy_nonoverlapping(s.as_ptr().add(first), p, s.len() - first);
        }
    }

    fn copy_out(&self, c: usize, start: usize, d: &mut [f32]) {
        let cap = self.cap() as usize;
        assert!(start <= cap && d.len() <= cap, "ring read out of bounds");
        let first = d.len().min(cap - start);
        let p = self.plane_ptr(c);
        // SAFETY: SPSC: the consumer reads only [read, read + n), published by the producer's release store.
        unsafe {
            core::ptr::copy_nonoverlapping(p.add(start), d.as_mut_ptr(), first);
            core::ptr::copy_nonoverlapping(p, d.as_mut_ptr().add(first), d.len() - first);
        }
    }
}

/// Byte offsets of an IO procedure's memfd (see `bifrost_audio_types::control::ShmLayout`), computed from the ring
/// sizes, each part 64-byte aligned.
pub fn layout(input_channels: u32, output_channels: u32, ring_frames: u32) -> bifrost_audio_types::control::ShmLayout {
    let activation = 0;
    let zero_stamp = activation + core::mem::size_of::<Activation>() as u32;
    let input_ring = zero_stamp + core::mem::size_of::<ZeroStamp>() as u32;
    let output_ring = input_ring + ring_bytes(input_channels, ring_frames);
    // The server's per-client "done" record follows the rings; the client's `finish` targets it.
    let done = output_ring + ring_bytes(output_channels, ring_frames);
    let total = done + core::mem::size_of::<Activation>() as u32;
    bifrost_audio_types::control::ShmLayout { activation, zero_stamp, input_ring, output_ring, ring_frames, total, done }
}

/// A shared memory region: a sealed memfd mapped read-write.
pub struct Region {
    pub fd: std::os::fd::OwnedFd,
    pub len: usize,
    /// Mapping address (as usize so the region is `Send`).
    pub addr: usize,
}

fn map_fd(fd: &std::os::fd::OwnedFd, len: usize) -> std::io::Result<usize> {
    use std::os::fd::AsRawFd;
    // SAFETY: a fresh shared mapping of a valid fd; the result is checked.
    let p = unsafe {
        libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0)
    };
    if p == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    Ok(p as usize)
}

impl Region {
    /// Creates a sealed (`F_SEAL_SHRINK | F_SEAL_GROW`) memfd of `len` bytes, zeroed and mapped.
    pub fn create(name: &str, len: usize) -> std::io::Result<Region> {
        use std::os::fd::{AsRawFd, FromRawFd};
        if len == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        }
        let cname = std::ffi::CString::new(name).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        // SAFETY: valid NUL-terminated name; the returned fd is owned right away.
        let raw = unsafe { libc::memfd_create(cname.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: raw is a fresh fd that nothing else owns.
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        // SAFETY: plain syscalls on our fd.
        unsafe {
            if libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK | libc::F_SEAL_GROW) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        let addr = map_fd(&fd, len)?;
        Ok(Region { fd, len, addr })
    }

    /// Maps a memfd received from the other side.
    pub fn map(fd: std::os::fd::OwnedFd, len: usize) -> std::io::Result<Region> {
        use std::os::fd::AsRawFd;
        // SAFETY: zeroed stat is a valid out buffer for fstat.
        let mut st: libc::stat = unsafe { core::mem::zeroed() };
        // SAFETY: valid fd and out pointer.
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Mapping beyond the file would SIGBUS on access; the peer must not be able to cause that.
        if len == 0 || (st.st_size as u64) < len as u64 {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        }
        // Without the shrink seal the peer could truncate the file under us.
        // SAFETY: plain syscall.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        if seals < 0 || seals & libc::F_SEAL_SHRINK == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        let addr = map_fd(&fd, len)?;
        Ok(Region { fd, len, addr })
    }

    /// A shared struct at `offset` (aligned, in bounds, checked).
    pub fn get<T>(&self, offset: u32) -> &T {
        let off = offset as usize;
        let size = core::mem::size_of::<T>();
        assert!(off.checked_add(size).is_some_and(|e| e <= self.len), "shm offset out of bounds");
        let p = self.addr + off;
        assert!(p % core::mem::align_of::<T>() == 0, "shm offset misaligned");
        // SAFETY: in bounds and aligned (checked); the mapping lives as long as the region; T is a shared atomic
        // struct for which every bit pattern is valid (zeroed at creation).
        unsafe { &*(p as *const T) }
    }

    /// Initialises the ring header at `offset` (producer side of the setup, before the fd is shared). Returns false,
    /// and writes nothing, when the offset is misaligned or the ring does not fit the mapping.
    pub fn init_ring(&self, offset: u32, channels: u32, frames: u32) -> bool {
        let off = offset as usize;
        if off % 64 != 0 || self.addr % 64 != 0 {
            return false;
        }
        if !off.checked_add(ring_bytes(channels, frames) as usize).is_some_and(|e| e <= self.len) {
            return false;
        }
        // SAFETY: in bounds, aligned; nobody else has the region yet, so the plain write of the non-atomic fields
        // does not race.
        unsafe {
            let h = (self.addr + off) as *mut RingHeader;
            core::ptr::addr_of_mut!((*h).frames).write(frames);
            core::ptr::addr_of_mut!((*h).channels).write(channels);
        }
        true
    }

    /// A ring view over the (initialised) ring at `offset`. Validates the header against the mapping, so a hostile
    /// peer cannot make the view run out of bounds.
    pub fn ring(&self, offset: u32) -> Option<Ring<'_>> {
        let off = offset as usize;
        if off % 64 != 0 || off.checked_add(64)? > self.len {
            return None;
        }
        let header: &RingHeader = self.get(offset);
        let cells = (header.channels as usize).checked_mul(header.frames as usize)?;
        let end = off.checked_add(64)?.checked_add(cells.checked_mul(4)?)?;
        if end > self.len {
            return None;
        }
        // SAFETY: bounds checked above; UnsafeCell<f32> has the layout of f32 and is 4-aligned (off + 64).
        let planes = unsafe { core::slice::from_raw_parts((self.addr + off + 64) as *const UnsafeCell<f32>, cells) };
        Ring::new(header, planes, header.channels, header.frames)
    }

    /// A ring view with a layout the caller trusts (`channels` planes of `frames`, as it laid the memfd out itself),
    /// ignoring whatever the peer wrote into the header's size fields.
    pub fn ring_trusted(&self, offset: u32, channels: u32, frames: u32) -> Option<Ring<'_>> {
        let off = offset as usize;
        if off % 64 != 0 || off.checked_add(64)? > self.len {
            return None;
        }
        let header: &RingHeader = self.get(offset);
        let cells = (channels as usize).checked_mul(frames as usize)?;
        let end = off.checked_add(64)?.checked_add(cells.checked_mul(4)?)?;
        if end > self.len {
            return None;
        }
        // SAFETY: bounds checked above; UnsafeCell<f32> has the layout of f32 and is 4-aligned (off + 64).
        let planes = unsafe { core::slice::from_raw_parts((self.addr + off + 64) as *const UnsafeCell<f32>, cells) };
        Ring::new(header, planes, channels, frames)
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: addr/len came from a successful mmap and are unmapped once.
        unsafe {
            libc::munmap(self.addr as *mut libc::c_void, self.len);
        }
    }
}

/// Host time in nanoseconds: `CLOCK_MONOTONIC_RAW` (vDSO, no syscall), the one clock every audio crate shares (the
/// contract's host clock; the window server presents on it). `bifrost-audio-types` is `no_std` without libc, so the
/// single implementation lives here and the HAL, server, ALSA driver and units re-export or call it.
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid out pointer.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Futex wait and wake on a word in shared memory (`FUTEX_WAIT`/`FUTEX_WAKE`, not private: cross-process).
pub mod futex {
    use core::sync::atomic::AtomicU32;

    /// Sleeps while `*word == expected`, at most `timeout_ns` (0 = forever). Returns false on timeout. Spurious
    /// returns (value changed, signal) return true; callers re-check their condition.
    pub fn wait(word: &AtomicU32, expected: u32, timeout_ns: u64) -> bool {
        let ts = libc::timespec { tv_sec: (timeout_ns / 1_000_000_000) as _, tv_nsec: (timeout_ns % 1_000_000_000) as _ };
        let tp: *const libc::timespec = if timeout_ns == 0 { core::ptr::null() } else { &ts };
        // SAFETY: the futex syscall with a valid shared word and timeout pointer; no memory is written.
        let r = unsafe { libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAIT, expected, tp) };
        !(r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ETIMEDOUT))
    }

    /// Wakes up to `n` sleepers.
    pub fn wake(word: &AtomicU32, n: u32) {
        // SAFETY: the futex syscall on a valid shared word.
        unsafe { libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAKE, n.min(i32::MAX as u32) as i32) };
    }
}

/// An eventfd for clients that wait in epoll (the counterpart of the futex word).
#[derive(Debug)]
pub struct EventFd {
    pub fd: std::os::fd::OwnedFd,
}

impl EventFd {
    /// A non-blocking, close-on-exec eventfd with counter 0.
    pub fn new() -> std::io::Result<EventFd> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain syscall; the fd is owned right away.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fresh fd.
        Ok(EventFd { fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) } })
    }

    /// Adds 1 to the counter, waking an epoll waiter (one `write` syscall).
    pub fn signal(&self) {
        use std::os::fd::AsRawFd;
        let one = 1u64.to_ne_bytes();
        // SAFETY: valid fd and an 8-byte buffer. A full counter (EAGAIN) already means "signalled".
        unsafe { libc::write(self.fd.as_raw_fd(), one.as_ptr().cast(), 8) };
    }

    /// Reads and clears the counter; 0 if it was not signalled.
    pub fn drain(&self) -> u64 {
        use std::os::fd::AsRawFd;
        let mut b = [0u8; 8];
        // SAFETY: valid fd and an 8-byte buffer.
        let r = unsafe { libc::read(self.fd.as_raw_fd(), b.as_mut_ptr().cast(), 8) };
        if r == 8 {
            u64::from_ne_bytes(b)
        } else {
            0
        }
    }
}

/// Wakes a node: bumps its futex word and wakes the sleeper.
fn wake_node(a: &Activation) {
    a.wake.fetch_add(1, Ordering::Release);
    futex::wake(&a.wake, 1);
}

impl Activation {
    /// Marks the node triggered and wakes it (used by `finish` and by the driver for nodes without inputs).
    pub fn trigger(&self, now_ns: u64) {
        self.signal_ns.store(now_ns, Ordering::Relaxed);
        self.status.store(STATUS_TRIGGERED, Ordering::Release);
        wake_node(self);
    }

    /// Sleeps until the node is triggered (status TRIGGERED -> AWAKE). `timeout_ns` 0 waits forever. Returns false on
    /// timeout. No lost wakeup: the futex word is read before the status check, so a trigger in between changes it
    /// and the futex wait returns at once.
    pub fn wait_triggered(&self, timeout_ns: u64) -> bool {
        let start = if timeout_ns != 0 { now_ns() } else { 0 };
        loop {
            let w = self.wake.load(Ordering::Acquire);
            if self.status.compare_exchange(STATUS_TRIGGERED, STATUS_AWAKE, Ordering::AcqRel, Ordering::Acquire).is_ok()
            {
                return true;
            }
            let left = if timeout_ns == 0 {
                0
            } else {
                let el = now_ns().saturating_sub(start);
                if el >= timeout_ns {
                    return false;
                }
                timeout_ns - el
            };
            futex::wait(&self.wake, w, left);
        }
    }
}

/// The graph trigger (PipeWire's scheme): called by a node when it finished. Decrements each target's `pending` and
/// wakes the targets that reach 0 (status TRIGGERED, signal time stamped, futex woken). Lock-free; a target is
/// woken exactly once per cycle.
pub fn finish(me: &Activation, targets: &[&Activation], now_ns: u64) {
    me.finish_ns.store(now_ns, Ordering::Relaxed);
    me.status.store(STATUS_FINISHED, Ordering::Release);
    for t in targets {
        // Saturating: a target already at 0 (a misbehaving client signalling twice) is never underflowed or re-woken.
        let hit = t.pending.fetch_update(Ordering::AcqRel, Ordering::Acquire, |p| p.checked_sub(1));
        if hit == Ok(1) {
            t.trigger(now_ns);
        }
    }
}

/// Resets a node for a new cycle: `pending = required`, status IDLE.
pub fn reset(node: &Activation, quantum: u32, rate: u32) {
    node.quantum.store(quantum, Ordering::Relaxed);
    node.rate.store(rate, Ordering::Relaxed);
    node.status.store(STATUS_IDLE, Ordering::Relaxed);
    node.pending.store(node.required, Ordering::Release);
}

/// Cross-process self test, shared by the unit test and `examples/chain.rs`.
pub mod selftest {
    use super::*;

    #[repr(C)]
    struct Ctl {
        stop: AtomicU32,
        _pad: u32,
        runs: [AtomicU64; 2],
    }

    /// Forks two followers A and B; the parent is the driver. For `duration_ms` it runs cycles D -> A -> B -> D
    /// (one every `period_us`), and returns the number of cycles. Errors if any cycle misses (1 s timeout) or a
    /// follower's run count differs from the cycle count.
    pub fn chain_fork(duration_ms: u64, period_us: u64) -> Result<u64, String> {
        let acts = core::mem::size_of::<Activation>();
        let region = Region::create("bifrost-audio-selftest", acts * 3 + 64).map_err(|e| e.to_string())?;
        let act = |i: u32| -> &Activation { region.get(i * acts as u32) };
        let ctl: &Ctl = region.get(acts as u32 * 3);
        // required: D 1 (from B), A 1 (from D), B 1 (from A)
        for i in 0..3 {
            // SAFETY: setup before fork; nobody else has the region; the pointer comes from the mapping address.
            unsafe { core::ptr::addr_of_mut!((*((region.addr + i as usize * acts) as *mut Activation)).required).write(1) };
        }
        let mut pids = Vec::new();
        for who in 0..2u32 {
            // SAFETY: the child only uses the shared mapping and `_exit`; no allocation after fork.
            let pid = unsafe { libc::fork() };
            if pid < 0 {
                return Err("fork failed".into());
            }
            if pid == 0 {
                let me = act(1 + who);
                let next = act(if who == 0 { 2 } else { 0 });
                loop {
                    if !me.wait_triggered(2_000_000_000) {
                        // SAFETY: leave the child without running destructors.
                        unsafe { libc::_exit(2) };
                    }
                    if ctl.stop.load(Ordering::Acquire) != 0 {
                        // SAFETY: as above.
                        unsafe { libc::_exit(0) };
                    }
                    ctl.runs[who as usize].fetch_add(1, Ordering::Relaxed);
                    finish(me, &[next], now_ns());
                }
            }
            pids.push(pid);
        }
        let d = act(0);
        let start = now_ns();
        let mut cycles = 0u64;
        let mut err = None;
        while now_ns() - start < duration_ms * 1_000_000 {
            for i in 0..3 {
                reset(act(i), 64, 48_000);
            }
            finish(d, &[act(1)], now_ns());
            if !d.wait_triggered(1_000_000_000) {
                err = Some(format!("missed cycle {cycles}"));
                break;
            }
            cycles += 1;
            std::thread::sleep(std::time::Duration::from_micros(period_us));
        }
        ctl.stop.store(1, Ordering::Release);
        for i in 0..3 {
            reset(act(i), 64, 48_000);
        }
        // Wake both followers: A to see `stop`, then B, which no longer gets a trigger from A. Triggering only A left
        // B waiting out its 2 s timeout, which exits with status 2 and failed the run.
        let now = now_ns();
        act(1).trigger(now);
        act(2).trigger(now);
        for pid in pids {
            let mut st = 0;
            // SAFETY: waiting for our own child.
            unsafe { libc::waitpid(pid, &mut st, 0) };
            if err.is_none() && !(libc::WIFEXITED(st) && libc::WEXITSTATUS(st) == 0) {
                err = Some(format!("follower {pid} exited with status {st}"));
            }
        }
        if err.is_none() {
            for w in 0..2 {
                let n = ctl.runs[w].load(Ordering::Relaxed);
                if n != cycles {
                    err = Some(format!("follower {w} ran {n} of {cycles} cycles"));
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(cycles),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};
    use std::sync::Arc;

    #[test]
    fn activation_layout_is_pinned() {
        assert_eq!(size_of::<Activation>(), 64);
        assert_eq!(align_of::<Activation>(), 64);
        assert_eq!(offset_of!(Activation, status), 0);
        assert_eq!(offset_of!(Activation, pending), 4);
        assert_eq!(offset_of!(Activation, required), 8);
        assert_eq!(offset_of!(Activation, xruns), 12);
        assert_eq!(offset_of!(Activation, signal_ns), 16);
        assert_eq!(offset_of!(Activation, awake_ns), 24);
        assert_eq!(offset_of!(Activation, finish_ns), 32);
        assert_eq!(offset_of!(Activation, quantum), 40);
        assert_eq!(offset_of!(Activation, rate), 44);
        assert_eq!(offset_of!(Activation, wake), 48);
    }

    #[test]
    fn zerostamp_and_ring_layout_is_pinned() {
        assert_eq!(size_of::<ZeroStamp>(), 64);
        assert_eq!(offset_of!(ZeroStamp, seq), 0);
        assert_eq!(offset_of!(ZeroStamp, seed), 4);
        assert_eq!(offset_of!(ZeroStamp, sample_time), 8);
        assert_eq!(offset_of!(ZeroStamp, host_ns), 16);
        assert_eq!(offset_of!(ZeroStamp, rate_scalar), 24);
        assert_eq!(size_of::<RingHeader>(), 64);
        assert_eq!(offset_of!(RingHeader, write), 0);
        assert_eq!(offset_of!(RingHeader, read), 8);
        assert_eq!(offset_of!(RingHeader, frames), 16);
        assert_eq!(offset_of!(RingHeader, channels), 20);
    }

    #[test]
    fn layout_offsets_by_hand() {
        // 2 in, 4 out, 256 frames: activation 0..64, stamp 64..128, input ring 64 + 2*256*4 = 2112 bytes from 128
        // to 2240; output ring 64 + 4*256*4 = 4160 bytes from 2240 to 6400.
        let l = layout(2, 4, 256);
        assert_eq!((l.activation, l.zero_stamp, l.input_ring, l.output_ring, l.total - 64), (0, 64, 128, 2240, 6400));
        // the done record follows the output ring: 6400..6464
        assert_eq!((l.done, l.total), (6400, 6464));
        assert_eq!(l.ring_frames, 256);
    }

    #[test]
    fn zerostamp_roundtrip() {
        let z = ZeroStamp::default();
        let v = ZeroStampValue { sample_time: 123_456, host_ns: 987, rate_scalar: 1.000_25, seed: 7 };
        z.publish(v);
        assert_eq!(z.read(), v);
        assert_eq!(z.seq.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn zerostamp_never_torn() {
        let z = Arc::new(ZeroStamp::default());
        let w = z.clone();
        let t = std::thread::spawn(move || {
            for i in 1..=300_000u64 {
                w.publish(ZeroStampValue { sample_time: i, host_ns: i * 2, rate_scalar: i as f64, seed: i as u32 });
            }
        });
        let mut reads = 0;
        while !t.is_finished() {
            let (v, consistent) = z.read_checked();
            if !consistent {
                continue; // retries exhausted against a hot writer: the value may be torn by definition
            }
            assert_eq!(v.host_ns, v.sample_time * 2);
            assert_eq!(v.rate_scalar, v.sample_time as f64);
            assert_eq!(v.seed, v.sample_time as u32);
            reads += 1;
        }
        t.join().unwrap();
        assert!(reads > 0);
    }

    fn ring_region(ch: u32, frames: u32) -> Region {
        let r = Region::create("t-ring", ring_bytes(ch, frames) as usize).unwrap();
        r.init_ring(0, ch, frames);
        r
    }

    #[test]
    fn ring_wraps_and_limits() {
        let r = ring_region(2, 8);
        let ring = r.ring(0).unwrap();
        assert_eq!((ring.readable(), ring.writable()), (0, 8));
        let a: Vec<f32> = (0..6).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..6).map(|i| 10.0 + i as f32).collect();
        assert_eq!(ring.write(&[&a, &b], 6), 6);
        assert_eq!(ring.write(&[&a, &b], 6), 2); // only 2 frames of space
        let (mut x, mut y) = ([0f32; 4], [0f32; 4]);
        assert_eq!(ring.read(&mut [&mut x, &mut y], 4), 4);
        assert_eq!(x, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(y, [10.0, 11.0, 12.0, 13.0]);
        // 4 left (frames 4,5 and the new 0,1); write 4 more to wrap across the end.
        assert_eq!(ring.write(&[&a[2..], &b[2..]], 4), 4);
        let (mut x, mut y) = ([0f32; 8], [0f32; 8]);
        assert_eq!(ring.read(&mut [&mut x, &mut y], 8), 8);
        assert_eq!(x, [4.0, 5.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(y[0], 14.0);
        assert_eq!(ring.readable(), 0);
    }

    #[test]
    fn ring_rejects_hostile_header() {
        let r = ring_region(2, 8);
        assert!(r.ring(4).is_none());
        let h: &RingHeader = r.get(0);
        h.write.store(1 << 40, Ordering::Relaxed);
        let ring = r.ring(0).unwrap();
        assert_eq!(ring.readable(), 8); // clamped
        // the producer side refuses a ring that does not fit the mapping, and writes nothing
        let big = Region::create("t-big", 128).unwrap();
        assert!(!big.init_ring(0, 4, 1000));
        // a header a peer wrote claiming more planes than the mapping holds
        // SAFETY: test-only plain writes to the non-atomic size fields of a live header, standing in for the peer.
        unsafe {
            let hp = big.addr as *mut RingHeader;
            core::ptr::addr_of_mut!((*hp).frames).write(1000);
            core::ptr::addr_of_mut!((*hp).channels).write(4);
        }
        assert!(big.ring(0).is_none());
    }

    #[test]
    fn ring_ignores_a_rewritten_header() {
        // The trusted view holds its own size; a peer rewriting the header's frames/channels changes nothing.
        let r = ring_region(2, 8);
        let ring = r.ring_trusted(0, 2, 8).unwrap();
        let h = r.addr as *mut RingHeader;
        // SAFETY: test-only plain writes to the non-atomic size fields of a live header, standing in for the peer.
        unsafe {
            core::ptr::addr_of_mut!((*h).frames).write(u32::MAX);
            core::ptr::addr_of_mut!((*h).channels).write(u32::MAX);
        }
        let a = [1.0f32; 6];
        let b = [2.0f32; 6];
        assert_eq!(ring.write(&[&a, &b], 6), 6);
        assert_eq!(ring.readable(), 6);
        assert_eq!(ring.write(&[&a, &b], 6), 2);
        let (mut x, mut y) = ([0f32; 8], [0f32; 8]);
        assert_eq!(ring.read(&mut [&mut x, &mut y], 8), 8);
        assert_eq!(&y[..6], &[2.0; 6]);
        // a trusted size larger than the mapping is refused
        assert!(r.ring_trusted(0, 2, 1 << 20).is_none());
    }

    #[test]
    fn stuck_odd_seqlock_does_not_spin_forever() {
        let z = ZeroStamp::default();
        z.publish(ZeroStampValue { sample_time: 5, host_ns: 6, rate_scalar: 1.0, seed: 1 });
        z.seq.store(3, Ordering::Relaxed); // the writer died mid-update
        let (v, consistent) = z.read_checked();
        assert!(!consistent);
        let _ = v;
        let _ = z.read();
    }

    #[test]
    fn ring_spsc_threads() {
        let r = Arc::new(ring_region(1, 64));
        let p = r.clone();
        let t = std::thread::spawn(move || {
            let ring = p.ring(0).unwrap();
            let mut next = 0u32;
            while next < 200_000 {
                let n = ring.writable().min(200_000 - next).min(37);
                let v: Vec<f32> = (next..next + n).map(|i| i as f32).collect();
                next += ring.write(&[&v], n);
            }
        });
        let ring = r.ring(0).unwrap();
        let mut want = 0u32;
        let mut buf = [0f32; 50];
        while want < 200_000 {
            let n = ring.read(&mut [&mut buf[..]], 50);
            for k in 0..n as usize {
                assert_eq!(buf[k], (want + k as u32) as f32);
            }
            want += n;
        }
        t.join().unwrap();
    }

    #[test]
    fn region_is_sealed_and_checked() {
        use std::os::fd::AsRawFd;
        let r = Region::create("t-seal", 4096).unwrap();
        // SAFETY: plain syscalls on our fd.
        unsafe {
            assert!(libc::fcntl(r.fd.as_raw_fd(), libc::F_GET_SEALS) & libc::F_SEAL_GROW != 0);
            assert_eq!(libc::ftruncate(r.fd.as_raw_fd(), 8192), -1);
        }
        let dup = r.fd.try_clone().unwrap();
        let m = Region::map(dup, 4096).unwrap();
        r.get::<AtomicU32>(100).store(5, Ordering::SeqCst);
        assert_eq!(m.get::<AtomicU32>(100).load(Ordering::SeqCst), 5);
        assert!(Region::map(r.fd.try_clone().unwrap(), 8192).is_err());
        assert!(std::panic::catch_unwind(|| r.get::<AtomicU64>(4090)).is_err());
        assert!(std::panic::catch_unwind(|| r.get::<AtomicU64>(4)).is_err());
    }

    #[test]
    fn futex_timeout_and_mismatch() {
        let w = AtomicU32::new(1);
        let t = now_ns();
        assert!(!futex::wait(&w, 1, 20_000_000));
        assert!(now_ns() - t >= 19_000_000);
        assert!(futex::wait(&w, 0, 0)); // value differs: returns at once
    }

    #[test]
    fn eventfd_signals() {
        let e = EventFd::new().unwrap();
        assert_eq!(e.drain(), 0);
        e.signal();
        e.signal();
        assert_eq!(e.drain(), 2);
        assert_eq!(e.drain(), 0);
    }

    #[test]
    fn finish_counts_down_and_wakes_once() {
        let a = Activation::default();
        let b = Activation::default();
        let t = Activation { required: 2, ..Default::default() };
        reset(&t, 64, 48_000);
        assert_eq!((t.pending.load(Ordering::Relaxed), t.quantum.load(Ordering::Relaxed), t.rate.load(Ordering::Relaxed)), (2, 64, 48_000));
        finish(&a, &[&t], 10);
        assert_eq!(t.status.load(Ordering::Relaxed), STATUS_IDLE);
        assert_eq!(a.status.load(Ordering::Relaxed), STATUS_FINISHED);
        assert_eq!(a.finish_ns.load(Ordering::Relaxed), 10);
        finish(&b, &[&t], 20);
        assert_eq!(t.status.load(Ordering::Relaxed), STATUS_TRIGGERED);
        assert_eq!(t.signal_ns.load(Ordering::Relaxed), 20);
        assert_eq!(t.wake.load(Ordering::Relaxed), 1);
        // a third (misbehaving) finish neither underflows nor wakes again
        finish(&a, &[&t], 30);
        assert_eq!(t.pending.load(Ordering::Relaxed), 0);
        assert_eq!(t.wake.load(Ordering::Relaxed), 1);
        assert!(t.wait_triggered(1_000_000));
        assert_eq!(t.status.load(Ordering::Relaxed), STATUS_AWAKE);
        assert!(!t.wait_triggered(5_000_000));
    }

    /// 8 threads: a driver, 6 parallel followers, one sink needing all 6. No lost wakeups (every wait has a timeout
    /// and the cycle counters must match) and no double triggers (a node must run exactly once per cycle).
    #[test]
    fn graph_stress_no_lost_wakeups_no_double_triggers() {
        const MID: usize = 6;
        const CYCLES: u64 = 100_000;
        let acts: Arc<Vec<Activation>> = Arc::new(
            (0..MID + 2)
                .map(|i| Activation { required: if i == 0 { 1 } else if i == MID + 1 { MID as u32 } else { 1 }, ..Default::default() })
                .collect(),
        );
        // 0 = driver, 1..=MID followers, MID+1 = sink. The sink signals the driver.
        let runs: Arc<Vec<AtomicU64>> = Arc::new((0..MID + 2).map(|_| AtomicU64::new(0)).collect());
        let mut hs = Vec::new();
        for i in 1..=MID + 1 {
            let (a, r) = (acts.clone(), runs.clone());
            hs.push(std::thread::spawn(move || {
                for _ in 0..CYCLES {
                    assert!(a[i].wait_triggered(5_000_000_000), "lost wakeup at node {i}");
                    r[i].fetch_add(1, Ordering::Relaxed);
                    if i == MID + 1 {
                        finish(&a[i], &[&a[0]], now_ns());
                    } else {
                        finish(&a[i], &[&a[MID + 1]], now_ns());
                    }
                }
            }));
        }
        let targets: Vec<&Activation> = acts[1..=MID].iter().collect();
        for c in 0..CYCLES {
            for a in acts.iter() {
                reset(a, 64, 48_000);
            }
            finish(&acts[0], &targets, now_ns());
            assert!(acts[0].wait_triggered(5_000_000_000), "driver starved at cycle {c}");
        }
        for h in hs {
            h.join().unwrap();
        }
        for i in 1..=MID + 1 {
            assert_eq!(runs[i].load(Ordering::Relaxed), CYCLES, "node {i}");
        }
    }

    #[test]
    fn cross_process_chain_has_no_missed_cycles() {
        // The 10 s run is `cargo run --example chain`; the unit test runs 1 s.
        let n = selftest::chain_fork(1_000, 200).unwrap();
        assert!(n > 100, "{n}");
    }
}
