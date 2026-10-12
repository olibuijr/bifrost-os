//! The ioplug callbacks and the entry point `_snd_pcm_bifrost_open`.

use crate::ffi::*;
use crate::ring::{Fmt, PcmRing};
use bifrost_audio_hal::{Device, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::{props, AudioError, BufferList, PropertyAddress, PropertyValue, Scope};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::ffi::{CStr, CString};
use std::sync::Arc;

const EIO: c_int = 5;
const EAGAIN: c_int = 11;
const ENOMEM: c_int = 12;
const EACCES: c_int = 13;
const EBUSY: c_int = 16;
const ENODEV: c_int = 19;
const EINVAL: c_int = 22;
const POLLIN: u16 = 1;
const POLLOUT: u16 = 4;
const POLLERR: u16 = 8;
const POLLHUP: u16 = 16;
const POLLNVAL: u16 = 32;

/// Unified Logging, subsystem `is.olibuijr.audio` (the plug-in lives in third-party processes, so it writes the
/// category-prefixed line to stderr).
fn log(category: &str, msg: &str) {
    eprintln!("is.olibuijr.audio/{category}: alsa plug-in: {msg}");
}

/// The negative errno alsa-lib gets for each failure.
pub fn errno_of(e: AudioError) -> c_int {
    match e {
        AudioError::NoServer | AudioError::DeviceGone | AudioError::UnknownObject => -ENODEV,
        AudioError::Hogged => -EBUSY,
        AudioError::PermissionDenied => -EACCES,
        AudioError::BadValue | AudioError::BadValueType | AudioError::UnknownProperty | AudioError::NotSettable => -EINVAL,
        AudioError::Protocol | AudioError::Io => -EIO,
    }
}

/// State shared by the application thread, the IO procedure and the wake thread.
pub struct Shared {
    pub ring: PcmRing,
    /// Bumped by the IO procedure each cycle; the wake thread waits on it.
    pub wake: AtomicU32,
    pub quit: AtomicBool,
    pub evfd: c_int,
}

fn futex(word: &AtomicU32, op: c_int, val: u32, timeout: *const libc::timespec) {
    // SAFETY: FUTEX_WAIT/WAKE only read the 32-bit word at `word`, which is a live atomic; `timeout` is null or
    // points to a live timespec. Errors (EAGAIN, EINTR, ETIMEDOUT) are expected and ignored by callers.
    unsafe {
        libc::syscall(libc::SYS_futex, word.as_ptr(), op | libc::FUTEX_PRIVATE_FLAG, val, timeout);
    }
}

impl Shared {
    /// Real-time safe: one atomic add and a futex wake.
    pub fn signal(&self) {
        self.wake.fetch_add(1, Ordering::Release);
        futex(&self.wake, libc::FUTEX_WAKE, 1, core::ptr::null());
    }

    fn kick_eventfd(&self) {
        let one: u64 = 1;
        // SAFETY: writes 8 bytes from a live local to an eventfd we own.
        unsafe { libc::write(self.evfd, &one as *const u64 as *const c_void, 8) };
    }

    fn drain_eventfd(&self) {
        let mut v: u64 = 0;
        // SAFETY: reads 8 bytes into a live local from a non-blocking eventfd we own.
        unsafe { libc::read(self.evfd, &mut v as *mut u64 as *mut c_void, 8) };
    }

    /// Wake thread body: sleeps on the futex, then signals the poll eventfd.
    fn wake_loop(&self) {
        let ts = libc::timespec { tv_sec: 0, tv_nsec: 100_000_000 };
        let mut seen = self.wake.load(Ordering::Acquire);
        while !self.quit.load(Ordering::Acquire) {
            futex(&self.wake, libc::FUTEX_WAIT, seen, &ts);
            let now = self.wake.load(Ordering::Acquire);
            if now != seen {
                seen = now;
                self.kick_eventfd();
            }
        }
    }
}

/// Which direction a plug-in instance serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Playback,
    Capture,
}

/// Spreads one interleaved frame over the buffer list (one buffer per channel, or interleaved groups).
pub fn scatter(out: &mut BufferList, i: usize, frame: &[f32]) {
    let mut c = 0;
    for b in out.buffers.iter_mut() {
        let n = b.channels.max(1) as usize;
        for k in 0..n {
            if let Some(s) = b.data.get_mut(i * n + k) {
                *s = frame.get(c).copied().unwrap_or(0.0);
            }
            c += 1;
        }
    }
}

/// Collects one interleaved frame from the buffer list (missing channels read as silence).
pub fn gather(inp: &BufferList, i: usize, frame: &mut [f32]) {
    let mut c = 0;
    for b in inp.buffers.iter() {
        let n = b.channels.max(1) as usize;
        for k in 0..n {
            if let Some(f) = frame.get_mut(c) {
                *f = b.data.get(i * n + k).copied().unwrap_or(0.0);
            }
            c += 1;
        }
    }
    for f in frame.iter_mut().skip(c) {
        *f = 0.0;
    }
}

fn frames_of(l: &BufferList) -> u32 {
    l.buffers.first().map(|b| (b.data.len() / b.channels.max(1) as usize) as u32).unwrap_or(0)
}

/// Address of sample `frame` of an ALSA channel area.
///
/// # Safety
/// `a.addr` must be a live mapping that covers the addressed byte range.
pub unsafe fn area_ptr(a: &ChannelArea, frame: u64) -> *mut u8 {
    let bits = a.first as u64 + a.step as u64 * frame;
    (a.addr as *mut u8).add((bits / 8) as usize)
}

/// Application to ring (playback `transfer`). Returns the frames taken.
///
/// # Safety
/// `areas` must point to `ring.channels()` areas that cover `size` frames from `offset` in `fmt`.
pub unsafe fn write_from_areas(ring: &PcmRing, fmt: Fmt, areas: *const ChannelArea, offset: u64, size: u32) -> u32 {
    let w = fmt.bytes();
    ring.push(size, |i, frame| {
        for (c, s) in frame.iter_mut().enumerate() {
            // SAFETY: the caller guarantees the area covers frame offset+i for channel c.
            let a = &*areas.add(c);
            let p = area_ptr(a, offset + i as u64);
            *s = fmt.decode(core::slice::from_raw_parts(p, w));
        }
    })
}

/// Ring to application (capture `transfer`). Returns the frames delivered; the rest of `size` is silence so the
/// application never sees stale data.
///
/// # Safety
/// Same as `write_from_areas`, writable.
pub unsafe fn read_to_areas(ring: &PcmRing, fmt: Fmt, areas: *const ChannelArea, offset: u64, size: u32) -> u32 {
    let w = fmt.bytes();
    let got = ring.pop(size, |i, frame| {
        for (c, s) in frame.iter().enumerate() {
            // SAFETY: as above.
            let a = &*areas.add(c);
            let p = area_ptr(a, offset + i as u64);
            fmt.encode(*s, core::slice::from_raw_parts_mut(p, w));
        }
    });
    for i in got..size {
        for c in 0..ring.channels() as usize {
            // SAFETY: as above.
            let a = &*areas.add(c);
            let p = area_ptr(a, offset + i as u64);
            fmt.encode(0.0, core::slice::from_raw_parts_mut(p, w));
        }
    }
    size
}

/// True when the ring lets the application make progress by `threshold` frames (see `ready_threshold`).
pub fn poll_ready(dir: Dir, ring: &PcmRing, threshold: u32) -> bool {
    match dir {
        Dir::Playback => ring.writable() >= threshold,
        Dir::Capture => ring.fill() >= threshold,
    }
}

/// The frames that make the PCM ready for the application: the sw_params `avail_min` when one was set, else one
/// period, kept within 1 and the ring size so a too-large value can never block forever.
pub fn ready_threshold(avail_min: u32, period: u32, ring_frames: u32) -> u32 {
    let want = if avail_min > 0 { avail_min } else { period };
    want.clamp(1, ring_frames.max(1))
}

/// What `poll_revents` reports. The poll descriptor is the eventfd the IO side signals (polled for POLLIN, which
/// is when the eventfd is readable); alsa-lib wants POLLOUT for a ready playback stream and POLLIN for a ready
/// capture stream, and POLLERR when the descriptor itself failed.
pub fn poll_mask(dir: Dir, ready: bool, fd_revents: u16) -> u16 {
    if fd_revents & (POLLERR | POLLHUP | POLLNVAL) != 0 {
        return POLLERR;
    }
    match (ready, dir) {
        (false, _) => 0,
        (true, Dir::Playback) => POLLOUT,
        (true, Dir::Capture) => POLLIN,
    }
}

/// What one look at the queue during `drain` decides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrainStep {
    /// Nothing left to wait for: capture, an empty queue, or a stream that is not running (paused or stopped),
    /// whose queued frames would never play.
    Done,
    /// Frames are still playing and the PCM is non-blocking: `-EAGAIN`, alsa-lib calls drain again.
    Again,
    /// Frames are still playing: wait for the IO procedure.
    Wait,
}

pub fn drain_step(dir: Dir, running: bool, fill: u32, nonblock: bool) -> DrainStep {
    if dir == Dir::Capture || fill == 0 || !running {
        DrainStep::Done
    } else if nonblock {
        DrainStep::Again
    } else {
        DrainStep::Wait
    }
}

/// Bounds a blocking drain: the whole budget (`drain_budget_ms`) and a stall limit, the longest the IO procedure
/// may go without consuming a frame before drain stops waiting for it (the server stopped calling it).
pub struct DrainWatch {
    budget_ms: u64,
    stall_ms: u64,
    waited_ms: u64,
    idle_ms: u64,
    last: u64,
}

impl DrainWatch {
    pub fn new(budget_ms: u64, stall_ms: u64, consumed: u64) -> DrainWatch {
        DrainWatch { budget_ms, stall_ms, waited_ms: 0, idle_ms: 0, last: consumed }
    }

    /// One millisecond passed and the consumer stands at `consumed`. False when drain should stop waiting.
    pub fn tick(&mut self, consumed: u64) -> bool {
        self.waited_ms += 1;
        if consumed != self.last {
            self.last = consumed;
            self.idle_ms = 0;
        } else {
            self.idle_ms += 1;
        }
        self.waited_ms < self.budget_ms && self.idle_ms < self.stall_ms
    }
}

/// The stall limit for `DrainWatch`: two periods of `period` frames at `rate`, plus 100 ms of scheduling slack.
pub fn drain_stall_ms(period: u32, rate: u32) -> u64 {
    2 * period as u64 * 1000 / rate.max(1) as u64 + 100
}

/// What `pause` does with the IO procedure: `Some(false)` stops it (pause), `Some(true)` starts it again (resume),
/// `None` when it already is in the wanted state. Neither touches the ring, so a resume continues at the same ring
/// position and `pointer` stays continuous across the pause.
pub fn pause_action(running: bool, enable: bool) -> Option<bool> {
    match (enable, running) {
        (true, true) => Some(false),
        (false, false) => Some(true),
        _ => None,
    }
}

/// Position the `pointer` callback reports: the consumed (playback) or produced (capture) frames in the buffer.
pub fn ring_pointer(dir: Dir, ring: &PcmRing) -> u64 {
    let pos = match dir {
        Dir::Playback => ring.consumed(),
        Dir::Capture => ring.written(),
    };
    pos % ring.frames().max(1) as u64
}

/// One open PCM. `io` must stay the first field: alsa-lib hands the callbacks `&mut io`.
#[repr(C)]
struct Bf {
    io: Ioplug,
    dir: Dir,
    name: CString,
    device: Device,
    dev_rate: u32,
    fmt: Fmt,
    evfd: c_int,
    shared: Option<Arc<Shared>>,
    proc_id: Option<IoProcId>,
    waker: Option<std::thread::JoinHandle<()>>,
    /// True while the IO procedure runs (after start or resume, until stop, pause or teardown). Atomic because
    /// alsa-lib releases the PCM lock around `drain`, so another thread may pause while drain reads it.
    running: AtomicBool,
    /// The sw_params `avail_min` (0 until sw_params ran), the poll readiness threshold.
    avail_min: u32,
    /// What the plug-in offered alsa-lib at open; hw_params checks the result against it.
    offer: Offer,
    /// The PERIOD_BYTES list handed to alsa-lib, kept alive with the PCM.
    period_bytes: Vec<c_uint>,
}

/// # Safety
/// `io` must be the `io` field of a live `Bf` (set as `private_data` at open).
unsafe fn bf<'a>(io: *mut Ioplug) -> &'a mut Bf {
    &mut *((*io).private_data as *mut Bf)
}

impl Bf {
    fn teardown(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(id) = self.proc_id.take() {
            let _ = self.device.stop(id);
            let _ = self.device.remove_io_proc(id);
        }
        if let Some(sh) = self.shared.take() {
            sh.quit.store(true, Ordering::Release);
            sh.signal();
        }
        if let Some(h) = self.waker.take() {
            let _ = h.join();
        }
    }
}

unsafe fn cb_start_impl(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    let (Some(id), Some(sh)) = (b.proc_id, b.shared.as_ref()) else { return -EINVAL };
    match b.device.start(id) {
        Ok(()) => {
            b.running.store(true, Ordering::Release);
            sh.kick_eventfd();
            0
        }
        Err(e) => {
            log("client", &format!("start failed: {e:?}"));
            errno_of(e)
        }
    }
}

unsafe fn cb_stop_impl(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    // The HAL ends the IO thread even when the server's reply fails, so the stream is stopped either way.
    b.running.store(false, Ordering::Release);
    let r = match b.proc_id {
        Some(id) => b.device.stop(id).map_or_else(errno_of, |()| 0),
        None => 0,
    };
    // Wake a poller so it sees the new state.
    if let Some(sh) = b.shared.as_ref() {
        sh.kick_eventfd();
    }
    r
}

unsafe fn cb_pointer_impl(io: *mut Ioplug) -> Sframes {
    let b = bf(io);
    match b.shared.as_ref() {
        Some(sh) => ring_pointer(b.dir, &sh.ring) as Sframes,
        None => 0,
    }
}

unsafe fn cb_transfer_impl(io: *mut Ioplug, areas: *const ChannelArea, offset: Uframes, size: Uframes) -> Sframes {
    let b = bf(io);
    let Some(sh) = b.shared.as_ref() else { return -(EINVAL as Sframes) };
    let n = match b.dir {
        Dir::Playback => write_from_areas(&sh.ring, b.fmt, areas, offset as u64, clamp_frames(size as u64)),
        Dir::Capture => read_to_areas(&sh.ring, b.fmt, areas, offset as u64, clamp_frames(size as u64)),
    };
    n as Sframes
}

unsafe fn cb_delay_impl(io: *mut Ioplug, delayp: *mut Sframes) -> c_int {
    let b = bf(io);
    // The ring delay: queued playback frames, or captured frames not yet read.
    *delayp = b.shared.as_ref().map(|s| s.ring.fill() as Sframes).unwrap_or(0);
    0
}

unsafe fn cb_hw_params_impl(io: *mut Ioplug, _p: *mut SndPcmHwParams) -> c_int {
    let b = bf(io);
    // alsa-lib fills format, channels, period and buffer in `io` before this call.
    let (format, channels, period, buffer) = (b.io.format as u32, b.io.channels, b.io.period_size as u64, b.io.buffer_size as u64);
    match b.offer.check(format, channels, period, buffer) {
        Ok(f) => {
            b.fmt = f;
            0
        }
        Err(e) => {
            log("client", &format!("hw_params refused: format {format}, {channels} channels, period {period}, buffer {buffer}"));
            e
        }
    }
}

unsafe fn cb_sw_params_impl(io: *mut Ioplug, p: *mut SndPcmSwParams) -> c_int {
    let b = bf(io);
    let mut v: Uframes = 0;
    if snd_pcm_sw_params_get_avail_min(p, &mut v) >= 0 {
        b.avail_min = v.min(u32::MAX as Uframes) as u32;
    }
    0
}

unsafe fn cb_hw_free_impl(io: *mut Ioplug) -> c_int {
    bf(io).teardown();
    0
}

unsafe fn cb_prepare_impl(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    let ioref = &*io;
    let ch = ioref.channels;
    let frames = match check_geometry(ioref.period_size as u64, ioref.buffer_size as u64, ch) {
        Ok(f) => f,
        Err(e) => return e,
    };
    // alsa-lib calls prepare after open, after every drop or drain and on every xrun recovery. With the same
    // geometry the IO procedure and wake thread stay: the stream is stopped, the ring is emptied and readiness reset.
    if let (Some(id), Some(sh)) = (b.proc_id, b.shared.clone()) {
        if sh.ring.frames() == frames && sh.ring.channels() == ch {
            b.running.store(false, Ordering::Release);
            let _ = b.device.stop(id);
            sh.ring.reset();
            sh.drain_eventfd();
            if b.dir == Dir::Playback {
                sh.kick_eventfd();
            }
            return 0;
        }
    }
    b.teardown();
    let sh = Arc::new(Shared { ring: PcmRing::new(frames, ch), wake: AtomicU32::new(0), quit: AtomicBool::new(false), evfd: b.evfd });
    // Clear a stale readiness count from the previous run. An empty playback ring is ready at once, and the IO
    // procedure does not run before start, so signal that now or a poller in the prepared state would wait forever.
    sh.drain_eventfd();
    if b.dir == Dir::Playback {
        sh.kick_eventfd();
    }
    let cfg = match b.dir {
        Dir::Playback => IoProcConfig { input_channels: 0, output_channels: ch, buffer_frames: 0 },
        Dir::Capture => IoProcConfig { input_channels: ch, output_channels: 0, buffer_frames: 0 },
    };
    let s2 = sh.clone();
    let proc: bifrost_audio_hal::IoProc = match b.dir {
        Dir::Playback => Box::new(move |_c, _now, _i, _its, out, _ots| {
            for buf in out.buffers.iter_mut() {
                buf.data.fill(0.0);
            }
            let want = frames_of(out);
            let got = s2.ring.pop(want, |i, f| scatter(out, i as usize, f));
            if got < want {
                s2.ring.xruns.fetch_add((want - got) as u64, Ordering::Relaxed);
            }
            s2.signal();
        }),
        Dir::Capture => Box::new(move |_c, _now, inp, _its, _out, _ots| {
            let have = frames_of(inp);
            let put = s2.ring.push(have, |i, f| gather(inp, i as usize, f));
            if put < have {
                s2.ring.xruns.fetch_add((have - put) as u64, Ordering::Relaxed);
            }
            s2.signal();
        }),
    };
    let id = match b.device.add_io_proc(cfg, proc) {
        Ok(id) => id,
        Err(e) => {
            log("client", &format!("add_io_proc failed: {e:?}"));
            return errno_of(e);
        }
    };
    let s3 = sh.clone();
    match std::thread::Builder::new().name("bifrost-alsa-wake".into()).spawn(move || s3.wake_loop()) {
        Ok(h) => b.waker = Some(h),
        Err(_) => {
            let _ = b.device.remove_io_proc(id);
            return -ENOMEM;
        }
    }
    b.proc_id = Some(id);
    b.shared = Some(sh);
    0
}

unsafe fn cb_drain_impl(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    let Some(sh) = b.shared.as_ref() else { return 0 };
    let nonblock = b.io.nonblock != 0;
    let period = b.io.period_size.min(u32::MAX as Uframes) as u32;
    // Wait for the IO procedure to play the queue, bounded by the buffer length plus slack and by a stall limit.
    let mut watch = DrainWatch::new(drain_budget_ms(sh.ring.frames(), b.dev_rate), drain_stall_ms(period, b.dev_rate), sh.ring.consumed());
    loop {
        match drain_step(b.dir, b.running.load(Ordering::Acquire), sh.ring.fill(), nonblock) {
            DrainStep::Done => return 0,
            DrainStep::Again => return -EAGAIN,
            DrainStep::Wait => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
        if !watch.tick(sh.ring.consumed()) {
            log("client", &format!("drain stopped waiting with {} frames queued", sh.ring.fill()));
            return 0;
        }
    }
}

unsafe fn cb_pause_impl(io: *mut Ioplug, enable: c_int) -> c_int {
    // alsa-lib's ioplug moves the state between RUNNING and PAUSED itself and calls this only for a valid move.
    match pause_action(bf(io).running.load(Ordering::Acquire), enable != 0) {
        Some(false) => cb_stop_impl(io),
        Some(true) => cb_start_impl(io),
        None => 0,
    }
}

unsafe fn cb_poll_revents_impl(io: *mut Ioplug, pfd: *mut Pollfd, nfds: c_uint, revents: *mut u16) -> c_int {
    let b = bf(io);
    if nfds != 1 {
        return -EINVAL;
    }
    let fd_revents = (*pfd).revents as u16;
    let ready = match b.shared.as_ref() {
        Some(sh) => {
            let t = ready_threshold(b.avail_min, b.io.period_size.min(u32::MAX as Uframes) as u32, sh.ring.frames());
            let mut ready = poll_ready(b.dir, &sh.ring, t);
            // Level-triggered: the eventfd stays readable while the PCM is ready and is emptied only when it is
            // not, so the next poll sleeps until the IO side signals. Look again after emptying it: the IO side
            // may have made room or delivered data in between, and its signal must not be lost.
            if !ready {
                sh.drain_eventfd();
                ready = poll_ready(b.dir, &sh.ring, t);
                if ready {
                    sh.kick_eventfd();
                }
            }
            ready
        }
        None => true,
    };
    *revents = poll_mask(b.dir, ready, fd_revents);
    0
}

unsafe fn cb_close_impl(io: *mut Ioplug) -> c_int {
    // SAFETY: private_data was produced by Box::into_raw at open and close runs once.
    let mut b = Box::from_raw((*io).private_data as *mut Bf);
    b.teardown();
    libc::close(b.evfd);
    0
}

/// Runs a callback body so a panic never unwinds into C: it becomes `-EIO`.
fn guard_i(f: impl FnOnce() -> c_int) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(-EIO)
}

/// As `guard_i` for callbacks that return a frame count.
fn guard_l(f: impl FnOnce() -> Sframes) -> Sframes {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(-(EIO as Sframes))
}

/// True when `io` and its private data are non-null, so `bf` is safe to call.
unsafe fn live(io: *mut Ioplug) -> bool {
    !io.is_null() && !(*io).private_data.is_null()
}

/// The largest ring (in frames) the plug-in accepts: 2^24 frames is over 5 minutes at 48 kHz.
pub const MAX_RING_FRAMES: u64 = 1 << 24;

/// Checks the geometry alsa-lib negotiated. Returns the ring size in frames.
pub fn check_geometry(period: u64, buffer: u64, channels: u32) -> Result<u32, c_int> {
    if channels == 0 || channels > 8 || buffer == 0 || buffer > MAX_RING_FRAMES || period > buffer {
        return Err(-EINVAL);
    }
    Ok(buffer as u32)
}

/// How long `drain` may wait for a ring of `frames` at `rate`: the buffer length plus 200 ms slack.
pub fn drain_budget_ms(frames: u32, rate: u32) -> u64 {
    frames as u64 * 1000 / rate.max(1) as u64 + 200
}

/// Clamps a transfer size to what the ring can take in one call.
pub fn clamp_frames(size: u64) -> u32 {
    size.min(MAX_RING_FRAMES) as u32
}

unsafe extern "C" fn cb_start(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_start_impl(io) } else { -EINVAL })
}
unsafe extern "C" fn cb_stop(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_stop_impl(io) } else { -EINVAL })
}
unsafe extern "C" fn cb_pointer(io: *mut Ioplug) -> Sframes {
    guard_l(|| if live(io) { cb_pointer_impl(io) } else { -(EINVAL as Sframes) })
}
unsafe extern "C" fn cb_transfer(io: *mut Ioplug, areas: *const ChannelArea, offset: Uframes, size: Uframes) -> Sframes {
    guard_l(|| if live(io) && !areas.is_null() { cb_transfer_impl(io, areas, offset, size) } else { -(EINVAL as Sframes) })
}
unsafe extern "C" fn cb_delay(io: *mut Ioplug, delayp: *mut Sframes) -> c_int {
    guard_i(|| if live(io) && !delayp.is_null() { cb_delay_impl(io, delayp) } else { -EINVAL })
}
unsafe extern "C" fn cb_hw_params(io: *mut Ioplug, p: *mut SndPcmHwParams) -> c_int {
    guard_i(|| if live(io) { cb_hw_params_impl(io, p) } else { -EINVAL })
}
unsafe extern "C" fn cb_hw_free(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_hw_free_impl(io) } else { -EINVAL })
}
unsafe extern "C" fn cb_sw_params(io: *mut Ioplug, p: *mut SndPcmSwParams) -> c_int {
    guard_i(|| if live(io) && !p.is_null() { cb_sw_params_impl(io, p) } else { -EINVAL })
}
unsafe extern "C" fn cb_prepare(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_prepare_impl(io) } else { -EINVAL })
}
unsafe extern "C" fn cb_drain(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_drain_impl(io) } else { -EINVAL })
}
unsafe extern "C" fn cb_pause(io: *mut Ioplug, enable: c_int) -> c_int {
    guard_i(|| if live(io) { cb_pause_impl(io, enable) } else { -EINVAL })
}
unsafe extern "C" fn cb_poll_revents(io: *mut Ioplug, pfd: *mut Pollfd, nfds: c_uint, revents: *mut u16) -> c_int {
    guard_i(|| if live(io) && !pfd.is_null() && !revents.is_null() { cb_poll_revents_impl(io, pfd, nfds, revents) } else { -EINVAL })
}
unsafe extern "C" fn cb_close(io: *mut Ioplug) -> c_int {
    guard_i(|| if live(io) { cb_close_impl(io) } else { 0 })
}

static CALLBACKS: IoplugCallback = IoplugCallback {
    start: Some(cb_start),
    stop: Some(cb_stop),
    pointer: Some(cb_pointer),
    transfer: Some(cb_transfer),
    close: Some(cb_close),
    hw_params: Some(cb_hw_params),
    hw_free: Some(cb_hw_free),
    sw_params: Some(cb_sw_params),
    prepare: Some(cb_prepare),
    drain: Some(cb_drain),
    pause: Some(cb_pause),
    resume: None,
    poll_descriptors_count: None,
    poll_descriptors: None,
    poll_revents: Some(cb_poll_revents),
    dump: None,
    delay: Some(cb_delay),
    query_chmaps: None,
    get_chmap: None,
    set_chmap: None,
};

/// The device's nominal rate, 48 kHz when the server does not say.
fn device_rate(d: &Device) -> u32 {
    match d.get(PropertyAddress::global(props::NOMINAL_RATE)) {
        Ok(PropertyValue::F64(r)) if r >= 8000.0 => r as u32,
        _ => 48_000,
    }
}

/// The client name: `client` from the configuration node, else `alsa:<process>`.
pub fn client_name(configured: Option<&str>, comm: &str) -> String {
    match configured {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => format!("alsa:{}", comm.trim()),
    }
}

unsafe fn config_client(conf: *mut SndConfig) -> Option<String> {
    if conf.is_null() {
        return None;
    }
    let mut it = snd_config_iterator_first(conf);
    let end = snd_config_iterator_end(conf);
    while it != end {
        let next = snd_config_iterator_next(it);
        let e = snd_config_iterator_entry(it);
        let mut id: *const c_char = core::ptr::null();
        if snd_config_get_id(e, &mut id) >= 0 && !id.is_null() && CStr::from_ptr(id).to_bytes() == b"client" && snd_config_get_type(e) == SND_CONFIG_TYPE_STRING {
            let mut s: *const c_char = core::ptr::null();
            if snd_config_get_string(e, &mut s) >= 0 && !s.is_null() {
                return Some(CStr::from_ptr(s).to_string_lossy().into_owned());
            }
        }
        it = next;
    }
    None
}

/// The device's channel count in the stream direction (the sum over its streams' virtual formats), when the
/// server reports one.
fn device_channels(d: &Device, dir: Dir) -> Option<u32> {
    let scope = if dir == Dir::Playback { Scope::Output } else { Scope::Input };
    let Ok(PropertyValue::Objects(streams)) = d.get(PropertyAddress::scoped(props::STREAMS, scope)) else { return None };
    let total: u32 = streams
        .iter()
        .filter_map(|&s| match d.system.get(s, PropertyAddress::global(props::VIRTUAL_FORMAT)) {
            Ok(PropertyValue::Format(f)) => Some(f.channels),
            _ => None,
        })
        .sum();
    (total > 0).then_some(total)
}

/// The device's current buffer size (the server quantum), when the server reports one.
fn server_quantum(d: &Device) -> Option<u32> {
    match d.get(PropertyAddress::global(props::BUFFER_FRAMES)) {
        Ok(PropertyValue::U32(n)) if n > 0 => Some(n),
        _ => None,
    }
}

/// The server's quantum limits (bifrost-audiod `MIN_QUANTUM`, `MAX_QUANTUM`, `DEFAULT_QUANTUM`).
pub const MIN_QUANTUM: u32 = 16;
pub const MAX_QUANTUM: u32 = 4096;
pub const DEFAULT_QUANTUM: u32 = 256;
/// The most channels the plug-in offers.
pub const MAX_OFFER_CHANNELS: u32 = 8;
/// Every offered format has 4-byte samples, so a frame has a fixed size and the period list in bytes is exact.
pub const SAMPLE_BYTES: u32 = 4;
/// The largest buffer offered: 65536 frames (1.37 s at 48 kHz).
pub const MAX_BUFFER_FRAMES: u32 = 1 << 16;
/// The most periods per buffer offered.
pub const MAX_PERIODS: u32 = 1024;

/// The constraint lists offered to alsa-lib, in ascending order (alsa-lib snaps interval ends onto list entries).
/// Formats are the 4-byte ones; `plug` in front of the plug-in converts S16 and every other format.
pub const FORMATS: [c_uint; 3] = [SND_PCM_FORMAT_S24_LE, SND_PCM_FORMAT_S32_LE, SND_PCM_FORMAT_FLOAT_LE];
pub const ACCESSES: [c_uint; 2] = [SND_PCM_ACCESS_MMAP_INTERLEAVED, SND_PCM_ACCESS_RW_INTERLEAVED];

/// The hardware configuration the plug-in offers alsa-lib, worked out at open from the server.
///
/// ioplug constrains periods and buffers in bytes only, so an exact frame constraint needs a fixed frame size:
/// one channel count (the device's) and 4-byte samples. Periods are the powers of two from the server's quantum
/// up to the largest quantum (4096): a period shorter than the quantum cannot wake the application more often,
/// since the IO procedure runs once per quantum, and alsa-lib's default choice (the smallest period) then lands on
/// the quantum, the preferred period.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offer {
    pub channels: u32,
    /// The smallest period in frames: the server quantum rounded up to a power of two in 16..=4096.
    pub period_min: u32,
}

impl Offer {
    pub fn new(device_channels: Option<u32>, server_quantum: Option<u32>) -> Offer {
        Offer {
            channels: device_channels.unwrap_or(2).clamp(1, MAX_OFFER_CHANNELS),
            period_min: server_quantum.unwrap_or(DEFAULT_QUANTUM).clamp(MIN_QUANTUM, MAX_QUANTUM).next_power_of_two(),
        }
    }

    pub fn frame_bytes(&self) -> u32 {
        SAMPLE_BYTES * self.channels
    }

    /// The offered period sizes in frames, ascending.
    pub fn periods(&self) -> Vec<u32> {
        let mut v = Vec::new();
        let mut p = self.period_min.max(1);
        while p <= MAX_QUANTUM {
            v.push(p);
            p *= 2;
        }
        v
    }

    /// The PERIOD_BYTES list, ascending.
    pub fn period_bytes(&self) -> Vec<c_uint> {
        self.periods().into_iter().map(|p| p * self.frame_bytes()).collect()
    }

    /// The BUFFER_BYTES range: two of the smallest periods up to `MAX_BUFFER_FRAMES`.
    pub fn buffer_bytes(&self) -> (c_uint, c_uint) {
        (2 * self.period_min * self.frame_bytes(), MAX_BUFFER_FRAMES * self.frame_bytes())
    }

    /// Checks what alsa-lib negotiated (hw_params) against the offer. Refinement keeps it inside already; this
    /// refuses anything else cleanly with `-EINVAL` instead of running with a geometry the server cannot serve.
    pub fn check(&self, format: u32, channels: u32, period: u64, buffer: u64) -> Result<Fmt, c_int> {
        let fmt = match Fmt::from_alsa(format) {
            Some(f) if FORMATS.contains(&format) => f,
            _ => return Err(-EINVAL),
        };
        let period_ok = period.is_power_of_two() && period >= self.period_min as u64 && period <= MAX_QUANTUM as u64;
        let buffer_ok = buffer >= 2 * period && buffer <= MAX_BUFFER_FRAMES as u64;
        if channels != self.channels || !period_ok || !buffer_ok {
            return Err(-EINVAL);
        }
        Ok(fmt)
    }
}

unsafe fn set_constraints(io: *mut Ioplug, rate: u32, offer: &Offer, period_bytes: &[c_uint]) -> c_int {
    let (bmin, bmax) = offer.buffer_bytes();
    let r = [
        snd_pcm_ioplug_set_param_list(io, SND_PCM_IOPLUG_HW_ACCESS, ACCESSES.len() as c_uint, ACCESSES.as_ptr()),
        snd_pcm_ioplug_set_param_list(io, SND_PCM_IOPLUG_HW_FORMAT, FORMATS.len() as c_uint, FORMATS.as_ptr()),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_CHANNELS, offer.channels, offer.channels),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_RATE, rate, rate),
        snd_pcm_ioplug_set_param_list(io, SND_PCM_IOPLUG_HW_PERIOD_BYTES, period_bytes.len() as c_uint, period_bytes.as_ptr()),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_BUFFER_BYTES, bmin, bmax),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_PERIODS, 2, MAX_PERIODS),
    ];
    r.into_iter().find(|&v| v < 0).unwrap_or(0)
}

/// The plug-in entry point alsa-lib resolves for `type bifrost` (`SND_PCM_PLUGIN_DEFINE_FUNC(bifrost)`).
///
/// # Safety
/// Called by alsa-lib with valid pointers.
#[no_mangle]
pub unsafe extern "C" fn _snd_pcm_bifrost_open(pcmp: *mut *mut SndPcm, name: *const c_char, root: *mut SndConfig, conf: *mut SndConfig, stream: c_int, mode: c_int) -> c_int {
    if pcmp.is_null() {
        return -EINVAL;
    }
    guard_i(|| open_impl(pcmp, name, root, conf, stream, mode))
}

unsafe fn open_impl(pcmp: *mut *mut SndPcm, name: *const c_char, _root: *mut SndConfig, conf: *mut SndConfig, stream: c_int, mode: c_int) -> c_int {
    let dir = if stream == SND_PCM_STREAM_PLAYBACK { Dir::Playback } else { Dir::Capture };
    let comm = std::fs::read_to_string("/proc/self/comm").unwrap_or_default();
    let client = client_name(config_client(conf).as_deref(), &comm);
    let sys = match System::connect(&client) {
        Ok(s) => s,
        Err(e) => {
            log("client", &format!("cannot reach bifrost-audiod: {e:?}"));
            return errno_of(e);
        }
    };
    let device = match sys.default_device(if dir == Dir::Playback { Role::Output } else { Role::Input }) {
        Ok(d) => d,
        Err(e) => {
            log("client", &format!("no default device: {e:?}"));
            return errno_of(e);
        }
    };
    let dev_rate = device_rate(&device);
    let offer = Offer::new(device_channels(&device, dir), server_quantum(&device));
    let evfd = libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC);
    if evfd < 0 {
        return -EIO;
    }
    let cname = CString::new(if name.is_null() { "bifrost".into() } else { CStr::from_ptr(name).to_string_lossy().into_owned() }).unwrap_or_default();
    let mut b = Box::new(Bf {
        io: core::mem::zeroed(),
        dir,
        name: cname,
        device,
        dev_rate,
        fmt: Fmt::F32,
        evfd,
        shared: None,
        proc_id: None,
        waker: None,
        running: AtomicBool::new(false),
        avail_min: 0,
        offer,
        period_bytes: offer.period_bytes(),
    });
    b.io.version = SND_PCM_IOPLUG_VERSION;
    b.io.name = b.name.as_ptr();
    b.io.flags = SND_PCM_IOPLUG_FLAG_LISTED | SND_PCM_IOPLUG_FLAG_MONOTONIC;
    // One poll descriptor in both directions: the eventfd, readable (POLLIN) when the IO side signalled.
    // `poll_revents` turns that into POLLOUT for playback. Polling an eventfd for POLLOUT would always succeed.
    b.io.poll_fd = evfd;
    b.io.poll_events = POLLIN as c_uint;
    b.io.mmap_rw = 0;
    b.io.callback = &CALLBACKS;
    let raw = Box::into_raw(b);
    (*raw).io.private_data = raw as *mut c_void;
    let io = &mut (*raw).io as *mut Ioplug;
    let err = snd_pcm_ioplug_create(io, (*raw).name.as_ptr(), stream, mode);
    if err < 0 {
        // SAFETY: create failed, so alsa-lib holds no reference; reclaim the box.
        let b = Box::from_raw(raw);
        libc::close(b.evfd);
        return err;
    }
    let err = set_constraints(io, dev_rate, &(*raw).offer, &(*raw).period_bytes);
    if err < 0 {
        snd_pcm_ioplug_delete(io);
        return err;
    }
    *pcmp = (*io).pcm;
    0
}

/// The symbol alsa-lib's `snd_dlsym_build_version` check looks for (`SND_PCM_DLSYM_VERSION`).
#[no_mangle]
#[used]
#[allow(non_upper_case_globals)]
pub static __snd_pcm_bifrost_open_dlsym_pcm_001: u8 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::buffer::Buffer;

    fn areas_for(bufs: &mut [Vec<u8>], bits: u32, interleaved_ch: Option<u32>) -> Vec<ChannelArea> {
        // Interleaved: all channels share bufs[0] with step = bits * channels; else one buffer per channel.
        match interleaved_ch {
            Some(ch) => (0..ch).map(|c| ChannelArea { addr: bufs[0].as_mut_ptr() as *mut c_void, first: bits * c, step: bits * ch }).collect(),
            None => bufs.iter_mut().map(|b| ChannelArea { addr: b.as_mut_ptr() as *mut c_void, first: 0, step: bits }).collect(),
        }
    }

    #[test]
    fn playback_transfer_interleaved_s16() {
        // Frames (L, R): (0x4000, 0xC000) = (0.5, -0.5), (0x0000, 0x2000) = (0, 0.25); read from offset 1 of 3.
        let mut data = vec![0u8; 3 * 4];
        data[4..8].copy_from_slice(&[0x00, 0x40, 0x00, 0xC0]);
        data[8..12].copy_from_slice(&[0x00, 0x00, 0x00, 0x20]);
        let mut v = vec![data];
        let areas = areas_for(&mut v, 16, Some(2));
        let ring = PcmRing::new(8, 2);
        // SAFETY: the areas cover 3 frames; we read frames 1..3.
        let n = unsafe { write_from_areas(&ring, Fmt::S16, areas.as_ptr(), 1, 2) };
        assert_eq!(n, 2);
        let mut got = vec![];
        ring.pop(2, |_, f| got.push((f[0], f[1])));
        assert_eq!(got, vec![(0.5, -0.5), (0.0, 0.25)]);
    }

    #[test]
    fn playback_transfer_stops_when_ring_full() {
        let mut v = vec![vec![0u8; 16]];
        let areas = areas_for(&mut v, 32, Some(1));
        let ring = PcmRing::new(3, 1);
        // SAFETY: 4 f32 frames available.
        assert_eq!(unsafe { write_from_areas(&ring, Fmt::F32, areas.as_ptr(), 0, 4) }, 3);
        assert_eq!(ring.writable(), 0);
    }

    #[test]
    fn capture_transfer_pads_with_silence() {
        let ring = PcmRing::new(8, 2);
        ring.push(1, |_, f| {
            f[0] = 0.5;
            f[1] = -0.5;
        });
        // non-interleaved areas, one buffer per channel, 2 frames of S16
        let mut v = vec![vec![0xAAu8; 4], vec![0xAAu8; 4]];
        let areas = areas_for(&mut v, 16, None);
        // SAFETY: 2 frames per channel.
        let n = unsafe { read_to_areas(&ring, Fmt::S16, areas.as_ptr(), 0, 2) };
        assert_eq!(n, 2);
        // 0.5 * 32767 = 16383.5, rounds to 16384 = 0x4000; second frame silent
        assert_eq!(v[0], vec![0x00, 0x40, 0x00, 0x00]);
        // -0.5 * 32767 = -16383.5, rounds away from zero to -16384 = 0xC000
        assert_eq!(v[1], vec![0x00, 0xC0, 0x00, 0x00]);
    }

    #[test]
    fn pointer_and_ready() {
        let ring = PcmRing::new(4, 1);
        ring.push(4, |_, _| {});
        assert_eq!(ring_pointer(Dir::Capture, &ring), 0); // 4 % 4
        assert_eq!(ring_pointer(Dir::Playback, &ring), 0);
        assert!(!poll_ready(Dir::Playback, &ring, 2)); // full
        assert!(poll_ready(Dir::Capture, &ring, 2));
        ring.pop(3, |_, _| {});
        assert_eq!(ring_pointer(Dir::Playback, &ring), 3);
        assert!(poll_ready(Dir::Playback, &ring, 3));
        assert!(!poll_ready(Dir::Playback, &ring, 4));
        assert!(!poll_ready(Dir::Capture, &ring, 2)); // one frame queued
    }

    #[test]
    fn scatter_gather_planar_and_grouped() {
        let mut l = BufferList { buffers: vec![Buffer { channels: 1, data: vec![0.0; 2] }, Buffer { channels: 1, data: vec![0.0; 2] }] };
        scatter(&mut l, 1, &[0.25, 0.75]);
        assert_eq!(l.buffers[0].data, vec![0.0, 0.25]);
        assert_eq!(l.buffers[1].data, vec![0.0, 0.75]);
        let mut f = [9.0; 3];
        gather(&l, 1, &mut f);
        assert_eq!(f, [0.25, 0.75, 0.0]);
        assert_eq!(frames_of(&l), 2);
        // one interleaved stereo buffer
        let mut g = BufferList { buffers: vec![Buffer { channels: 2, data: vec![0.0; 4] }] };
        scatter(&mut g, 1, &[1.0, 2.0]);
        assert_eq!(g.buffers[0].data, vec![0.0, 0.0, 1.0, 2.0]);
        assert_eq!(frames_of(&g), 2);
    }

    #[test]
    fn errno_mapping() {
        assert_eq!(errno_of(AudioError::NoServer), -19);
        assert_eq!(errno_of(AudioError::Hogged), -16);
        assert_eq!(errno_of(AudioError::PermissionDenied), -13);
        assert_eq!(errno_of(AudioError::BadValue), -22);
        assert_eq!(errno_of(AudioError::Io), -5);
    }

    #[test]
    fn client_names() {
        assert_eq!(client_name(None, "speaker-test\n"), "alsa:speaker-test");
        assert_eq!(client_name(Some("x"), "y"), "x");
        assert_eq!(client_name(Some(""), "y"), "alsa:y");
    }

    #[test]
    fn wake_thread_kicks_eventfd() {
        // SAFETY: plain eventfd creation.
        let evfd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        assert!(evfd >= 0);
        let sh = Arc::new(Shared { ring: PcmRing::new(4, 1), wake: AtomicU32::new(0), quit: AtomicBool::new(false), evfd });
        let s2 = sh.clone();
        let h = std::thread::spawn(move || s2.wake_loop());
        std::thread::sleep(std::time::Duration::from_millis(20));
        sh.signal();
        let mut pfd = libc::pollfd { fd: evfd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, 2000) };
        assert_eq!(r, 1);
        sh.quit.store(true, Ordering::Release);
        sh.signal();
        h.join().unwrap();
        // SAFETY: closing our own fd.
        unsafe { libc::close(evfd) };
    }

    #[test]
    fn guards_turn_panics_into_eio() {
        assert_eq!(guard_i(|| panic!("boom")), -5);
        assert_eq!(guard_l(|| panic!("boom")), -5);
        assert_eq!(guard_i(|| 7), 7);
        assert_eq!(guard_l(|| 9), 9);
    }

    #[test]
    fn null_pointers_are_rejected_before_any_dereference() {
        // SAFETY: every call passes null, which the entry points must reject without touching memory.
        unsafe {
            let null = core::ptr::null_mut::<Ioplug>();
            assert_eq!(cb_start(null), -EINVAL);
            assert_eq!(cb_stop(null), -EINVAL);
            assert_eq!(cb_pointer(null), -(EINVAL as Sframes));
            assert_eq!(cb_transfer(null, core::ptr::null(), 0, 1), -(EINVAL as Sframes));
            assert_eq!(cb_delay(null, core::ptr::null_mut()), -EINVAL);
            assert_eq!(cb_hw_params(null, core::ptr::null_mut()), -EINVAL);
            assert_eq!(cb_hw_free(null), -EINVAL);
            assert_eq!(cb_sw_params(null, core::ptr::null_mut()), -EINVAL);
            assert_eq!(cb_prepare(null), -EINVAL);
            assert_eq!(cb_drain(null), -EINVAL);
            assert_eq!(cb_pause(null, 1), -EINVAL);
            assert_eq!(cb_poll_revents(null, core::ptr::null_mut(), 1, core::ptr::null_mut()), -EINVAL);
            assert_eq!(cb_close(null), 0);
            assert_eq!(_snd_pcm_bifrost_open(core::ptr::null_mut(), core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), 0, 0), -EINVAL);
            // An Ioplug with no private data is also not live.
            let mut io: Ioplug = core::mem::zeroed();
            assert_eq!(cb_start(&mut io), -EINVAL);
            assert_eq!(cb_close(&mut io), 0);
        }
    }

    #[test]
    fn geometry_checks() {
        assert_eq!(check_geometry(1024, 4096, 2), Ok(4096));
        assert_eq!(check_geometry(0, 4096, 2), Ok(4096));
        assert_eq!(check_geometry(4096, 4096, 8), Ok(4096));
        assert_eq!(check_geometry(1024, 0, 2), Err(-EINVAL));
        assert_eq!(check_geometry(1024, 4096, 0), Err(-EINVAL));
        assert_eq!(check_geometry(1024, 4096, 9), Err(-EINVAL));
        assert_eq!(check_geometry(8192, 4096, 2), Err(-EINVAL));
        assert_eq!(check_geometry(1024, MAX_RING_FRAMES, 2), Ok(1 << 24));
        assert_eq!(check_geometry(1024, MAX_RING_FRAMES + 1, 2), Err(-EINVAL));
        // A 32-bit truncation that would give 0 is caught.
        assert_eq!(check_geometry(1, 1 << 32, 2), Err(-EINVAL));
    }

    #[test]
    fn drain_budget_and_clamp() {
        // 48000 frames at 48 kHz is 1000 ms plus 200 ms.
        assert_eq!(drain_budget_ms(48_000, 48_000), 1200);
        // A zero rate does not divide by zero: 100 frames * 1000 / 1 + 200.
        assert_eq!(drain_budget_ms(100, 0), 100_200);
        assert_eq!(clamp_frames(100), 100);
        assert_eq!(clamp_frames(u64::MAX), 1 << 24);
    }

    #[test]
    fn area_ptr_handles_bit_offsets() {
        let mut buf = [0u8; 16];
        let a = ChannelArea { addr: buf.as_mut_ptr() as *mut c_void, first: 16, step: 32 };
        // SAFETY: addresses stay inside the 16-byte buffer (2 + 4*3 = 14).
        let off = unsafe { area_ptr(&a, 3).offset_from(buf.as_mut_ptr()) };
        assert_eq!(off, 14);
    }

    #[test]
    fn constraint_tables() {
        // S24_LE, S32_LE, FLOAT_LE: ascending, all 4-byte samples.
        assert_eq!(FORMATS, [6, 10, 14]);
        assert_eq!(ACCESSES, [0, 3]);
    }

    #[test]
    fn offer_follows_the_server() {
        // Stereo device, quantum 256: periods 256..4096, 8-byte frames.
        let o = Offer::new(Some(2), Some(256));
        assert_eq!(o, Offer { channels: 2, period_min: 256 });
        assert_eq!(o.periods(), vec![256, 512, 1024, 2048, 4096]);
        assert_eq!(o.frame_bytes(), 8);
        assert_eq!(o.period_bytes(), vec![2048, 4096, 8192, 16384, 32768]);
        // 2 * 256 * 8 = 4096; 65536 * 8 = 524288
        assert_eq!(o.buffer_bytes(), (4096, 524_288));
        // Nothing reported: stereo at the default quantum.
        assert_eq!(Offer::new(None, None), Offer { channels: 2, period_min: 256 });
        // 12 channels clamp to 8; quantum 300 rounds up to 512.
        assert_eq!(Offer::new(Some(12), Some(300)), Offer { channels: 8, period_min: 512 });
        // Quantum 5 clamps to 16: nine periods 16, 32, ..., 4096.
        let small = Offer::new(Some(1), Some(5));
        assert_eq!(small, Offer { channels: 1, period_min: 16 });
        assert_eq!(small.periods().len(), 9);
        assert_eq!(small.periods().first(), Some(&16));
        assert_eq!(small.periods().last(), Some(&4096));
        assert_eq!(small.period_bytes()[0], 64);
        // Quantum 10000 clamps to 4096: one period.
        assert_eq!(Offer::new(Some(0), Some(10_000)), Offer { channels: 1, period_min: 4096 });
        assert_eq!(Offer::new(Some(2), Some(10_000)).periods(), vec![4096]);
    }

    #[test]
    fn hw_params_check_refuses_what_the_server_cannot_serve() {
        let o = Offer::new(Some(2), Some(256));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 256, 1024), Ok(Fmt::F32));
        assert_eq!(o.check(SND_PCM_FORMAT_S24_LE, 2, 512, 2048), Ok(Fmt::S24));
        assert_eq!(o.check(SND_PCM_FORMAT_S32_LE, 2, 4096, 65536), Ok(Fmt::S32));
        // S16 and unknown formats go through `plug`, not straight to the plug-in.
        assert_eq!(o.check(SND_PCM_FORMAT_S16_LE, 2, 256, 1024), Err(-EINVAL));
        assert_eq!(o.check(99, 2, 256, 1024), Err(-EINVAL));
        assert_eq!(o.check(u32::MAX, 2, 256, 1024), Err(-EINVAL));
        // Another channel count than offered.
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 1, 256, 1024), Err(-EINVAL));
        // Below the quantum, not a power of two, above 4096.
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 128, 1024), Err(-EINVAL));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 384, 1536), Err(-EINVAL));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 8192, 16384), Err(-EINVAL));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 0, 1024), Err(-EINVAL));
        // Buffer shorter than two periods or longer than 65536 frames.
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 256, 256), Err(-EINVAL));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 256, 512), Ok(Fmt::F32));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 256, 65536), Ok(Fmt::F32));
        assert_eq!(o.check(SND_PCM_FORMAT_FLOAT_LE, 2, 256, 65537), Err(-EINVAL));
    }

    #[test]
    fn drain_returns_at_once_unless_frames_are_playing() {
        use DrainStep::*;
        // Stopped or paused with frames queued: they would never play, so no wait.
        assert_eq!(drain_step(Dir::Playback, false, 512, false), Done);
        assert_eq!(drain_step(Dir::Playback, true, 0, false), Done);
        assert_eq!(drain_step(Dir::Capture, true, 512, false), Done);
        assert_eq!(drain_step(Dir::Playback, true, 512, false), Wait);
        assert_eq!(drain_step(Dir::Playback, true, 512, true), Again);
        assert_eq!(drain_step(Dir::Playback, false, 512, true), Done);
    }

    #[test]
    fn drain_watch_stops_on_stall_and_budget() {
        // No progress: stall limit 3 ends the wait on the third tick.
        let mut w = DrainWatch::new(10, 3, 0);
        assert!(w.tick(0));
        assert!(w.tick(0));
        assert!(!w.tick(0));
        // Steady progress: the budget of 5 ms ends it on the fifth tick.
        let mut w = DrainWatch::new(5, 100, 0);
        assert!(w.tick(1));
        assert!(w.tick(2));
        assert!(w.tick(3));
        assert!(w.tick(4));
        assert!(!w.tick(5));
        // Progress resets the stall count: idle 2, then progress, then idle 2 again stays under 3.
        let mut w = DrainWatch::new(100, 3, 7);
        assert!(w.tick(7));
        assert!(w.tick(7));
        assert!(w.tick(8));
        assert!(w.tick(8));
        assert!(w.tick(8));
        assert!(!w.tick(8));
        // 2 * 256 * 1000 / 48000 = 10 (integer), plus 100.
        assert_eq!(drain_stall_ms(256, 48_000), 110);
        // 2 * 4096 * 1000 / 8000 = 1024, plus 100.
        assert_eq!(drain_stall_ms(4096, 8_000), 1124);
        // A zero rate does not divide by zero.
        assert_eq!(drain_stall_ms(16, 0), 32_100);
    }

    #[test]
    fn pause_and_resume_keep_the_ring_position() {
        assert_eq!(pause_action(true, true), Some(false));
        assert_eq!(pause_action(false, true), None);
        assert_eq!(pause_action(false, false), Some(true));
        assert_eq!(pause_action(true, false), None);
        // Neither pause nor resume touches the ring: the pointer before and after is the same.
        let ring = PcmRing::new(8, 1);
        ring.push(6, |_, _| {});
        ring.pop(3, |_, _| {});
        let before = ring_pointer(Dir::Playback, &ring);
        assert_eq!(before, 3);
        assert_eq!(ring.fill(), 3);
        // (pause: the IO procedure stops; resume: it starts again; the ring is the same object)
        assert_eq!(ring_pointer(Dir::Playback, &ring), before);
        ring.pop(3, |_, _| {});
        assert_eq!(ring_pointer(Dir::Playback, &ring), 6);
    }

    #[test]
    fn poll_revents_translation() {
        // The eventfd is readable (POLLIN); playback reports POLLOUT, capture POLLIN.
        assert_eq!(poll_mask(Dir::Playback, true, POLLIN), POLLOUT);
        assert_eq!(poll_mask(Dir::Capture, true, POLLIN), POLLIN);
        assert_eq!(poll_mask(Dir::Playback, false, POLLIN), 0);
        assert_eq!(poll_mask(Dir::Capture, false, POLLIN), 0);
        // Readiness comes from the ring, not from the descriptor alone.
        assert_eq!(poll_mask(Dir::Playback, true, 0), POLLOUT);
        // A failed descriptor is an error whatever the ring says.
        assert_eq!(poll_mask(Dir::Capture, true, POLLHUP), POLLERR);
        assert_eq!(poll_mask(Dir::Playback, false, POLLERR | POLLIN), POLLERR);
        assert_eq!(poll_mask(Dir::Playback, true, POLLNVAL), POLLERR);
        assert_eq!((POLLIN, POLLOUT, POLLERR, POLLHUP, POLLNVAL), (1, 4, 8, 16, 32));
    }

    #[test]
    fn ready_threshold_uses_avail_min() {
        assert_eq!(ready_threshold(0, 256, 1024), 256);
        assert_eq!(ready_threshold(100, 256, 1024), 100);
        assert_eq!(ready_threshold(5000, 256, 1024), 1024);
        assert_eq!(ready_threshold(0, 0, 0), 1);
        // Playback ring of 8 with 5 queued: 3 writable, ready at avail_min 3, not at 4.
        let ring = PcmRing::new(8, 1);
        ring.push(5, |_, _| {});
        assert!(poll_ready(Dir::Playback, &ring, ready_threshold(3, 4, 8)));
        assert!(!poll_ready(Dir::Playback, &ring, ready_threshold(0, 4, 8)));
        assert!(poll_ready(Dir::Capture, &ring, ready_threshold(0, 4, 8)));
    }

    #[test]
    fn eventfd_stays_readable_while_ready() {
        // The level-triggered rule `poll_revents` follows: kick once, the descriptor stays readable until drained.
        // SAFETY: plain eventfd creation.
        let evfd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        assert!(evfd >= 0);
        let sh = Shared { ring: PcmRing::new(4, 1), wake: AtomicU32::new(0), quit: AtomicBool::new(false), evfd };
        let readable = || {
            let mut pfd = libc::pollfd { fd: evfd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd, zero timeout.
            unsafe { libc::poll(&mut pfd, 1, 0) == 1 }
        };
        assert!(!readable());
        sh.kick_eventfd();
        assert!(readable());
        assert!(readable());
        sh.drain_eventfd();
        assert!(!readable());
        // SAFETY: closing our own fd.
        unsafe { libc::close(evfd) };
    }
}
