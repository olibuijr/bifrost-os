//! The ioplug callbacks and the entry point `_snd_pcm_bifrost_open`.

use crate::ffi::*;
use crate::ring::{Fmt, PcmRing};
use bifrost_audio_hal::{Device, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::{props, AudioError, BufferList, PropertyAddress, PropertyValue};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::ffi::{CStr, CString};
use std::sync::Arc;

const EIO: c_int = 5;
const ENOMEM: c_int = 12;
const EACCES: c_int = 13;
const EBUSY: c_int = 16;
const ENODEV: c_int = 19;
const EINVAL: c_int = 22;
const POLLIN: u16 = 1;
const POLLOUT: u16 = 4;

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

/// True when the ring lets the application make progress by one period.
pub fn poll_ready(dir: Dir, ring: &PcmRing, period: u32) -> bool {
    match dir {
        Dir::Playback => ring.writable() >= period,
        Dir::Capture => ring.fill() >= period,
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
}

/// # Safety
/// `io` must be the `io` field of a live `Bf` (set as `private_data` at open).
unsafe fn bf<'a>(io: *mut Ioplug) -> &'a mut Bf {
    &mut *((*io).private_data as *mut Bf)
}

impl Bf {
    fn teardown(&mut self) {
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

unsafe extern "C" fn cb_start(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    let (Some(id), Some(sh)) = (b.proc_id, b.shared.as_ref()) else { return -EINVAL };
    match b.device.start(id) {
        Ok(()) => {
            sh.kick_eventfd();
            0
        }
        Err(e) => {
            log("client", &format!("start failed: {e:?}"));
            errno_of(e)
        }
    }
}

unsafe extern "C" fn cb_stop(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    if let Some(id) = b.proc_id {
        if let Err(e) = b.device.stop(id) {
            return errno_of(e);
        }
    }
    if let Some(sh) = b.shared.as_ref() {
        sh.kick_eventfd();
    }
    0
}

unsafe extern "C" fn cb_pointer(io: *mut Ioplug) -> Sframes {
    let b = bf(io);
    match b.shared.as_ref() {
        Some(sh) => ring_pointer(b.dir, &sh.ring) as Sframes,
        None => 0,
    }
}

unsafe extern "C" fn cb_transfer(io: *mut Ioplug, areas: *const ChannelArea, offset: Uframes, size: Uframes) -> Sframes {
    let b = bf(io);
    let Some(sh) = b.shared.as_ref() else { return -(EINVAL as Sframes) };
    let n = match b.dir {
        Dir::Playback => write_from_areas(&sh.ring, b.fmt, areas, offset as u64, size as u32),
        Dir::Capture => read_to_areas(&sh.ring, b.fmt, areas, offset as u64, size as u32),
    };
    n as Sframes
}

unsafe extern "C" fn cb_delay(io: *mut Ioplug, delayp: *mut Sframes) -> c_int {
    let b = bf(io);
    // The ring delay: queued playback frames, or captured frames not yet read.
    *delayp = b.shared.as_ref().map(|s| s.ring.fill() as Sframes).unwrap_or(0);
    0
}

unsafe extern "C" fn cb_hw_params(io: *mut Ioplug, _p: *mut SndPcmHwParams) -> c_int {
    let b = bf(io);
    match Fmt::from_alsa((*io).format as u32) {
        Some(f) => {
            b.fmt = f;
            0
        }
        None => -EINVAL,
    }
}

unsafe extern "C" fn cb_hw_free(io: *mut Ioplug) -> c_int {
    bf(io).teardown();
    0
}

unsafe extern "C" fn cb_prepare(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    b.teardown();
    let ioref = &*io;
    let (frames, ch) = (ioref.buffer_size as u32, ioref.channels);
    if frames == 0 || ch == 0 {
        return -EINVAL;
    }
    let sh = Arc::new(Shared { ring: PcmRing::new(frames, ch), wake: AtomicU32::new(0), quit: AtomicBool::new(false), evfd: b.evfd });
    // Clear a stale readiness count from the previous run.
    sh.drain_eventfd();
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

unsafe extern "C" fn cb_drain(io: *mut Ioplug) -> c_int {
    let b = bf(io);
    if b.dir == Dir::Playback {
        if let Some(sh) = b.shared.as_ref() {
            // Wait for the IO procedure to consume the queue, bounded by the buffer length plus slack.
            let ms = (sh.ring.frames() as u64 * 1000 / b.dev_rate.max(1) as u64) + 200;
            let mut waited = 0;
            while sh.ring.fill() > 0 && waited < ms {
                std::thread::sleep(std::time::Duration::from_millis(1));
                waited += 1;
            }
        }
    }
    0
}

unsafe extern "C" fn cb_pause(io: *mut Ioplug, enable: c_int) -> c_int {
    if enable != 0 {
        cb_stop(io)
    } else {
        cb_start(io)
    }
}

unsafe extern "C" fn cb_poll_revents(io: *mut Ioplug, pfd: *mut Pollfd, nfds: c_uint, revents: *mut u16) -> c_int {
    let b = bf(io);
    if nfds != 1 {
        return -EINVAL;
    }
    let events = (*pfd).events as u16;
    let ready = match b.shared.as_ref() {
        Some(sh) => {
            sh.drain_eventfd();
            poll_ready(b.dir, &sh.ring, (*io).period_size as u32)
        }
        None => true,
    };
    *revents = if ready { events & (POLLIN | POLLOUT) } else { 0 };
    0
}

unsafe extern "C" fn cb_close(io: *mut Ioplug) -> c_int {
    // SAFETY: private_data was produced by Box::into_raw at open and close runs once.
    let mut b = Box::from_raw((*io).private_data as *mut Bf);
    b.teardown();
    libc::close(b.evfd);
    0
}

static CALLBACKS: IoplugCallback = IoplugCallback {
    start: Some(cb_start),
    stop: Some(cb_stop),
    pointer: Some(cb_pointer),
    transfer: Some(cb_transfer),
    close: Some(cb_close),
    hw_params: Some(cb_hw_params),
    hw_free: Some(cb_hw_free),
    sw_params: None,
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

/// The constraint lists offered to alsa-lib: formats in preference order.
pub const FORMATS: [c_uint; 4] = [SND_PCM_FORMAT_S16_LE, SND_PCM_FORMAT_S24_LE, SND_PCM_FORMAT_S32_LE, SND_PCM_FORMAT_FLOAT_LE];
pub const ACCESSES: [c_uint; 2] = [SND_PCM_ACCESS_MMAP_INTERLEAVED, SND_PCM_ACCESS_RW_INTERLEAVED];

unsafe fn set_constraints(io: *mut Ioplug, rate: u32) -> c_int {
    let r = [
        snd_pcm_ioplug_set_param_list(io, SND_PCM_IOPLUG_HW_ACCESS, ACCESSES.len() as c_uint, ACCESSES.as_ptr()),
        snd_pcm_ioplug_set_param_list(io, SND_PCM_IOPLUG_HW_FORMAT, FORMATS.len() as c_uint, FORMATS.as_ptr()),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_CHANNELS, 1, 8),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_RATE, rate, rate),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_PERIOD_BYTES, 128, 2 * 1024 * 1024),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_BUFFER_BYTES, 256, 4 * 1024 * 1024),
        snd_pcm_ioplug_set_param_minmax(io, SND_PCM_IOPLUG_HW_PERIODS, 2, 1024),
    ];
    r.into_iter().find(|&v| v < 0).unwrap_or(0)
}

/// The plug-in entry point alsa-lib resolves for `type bifrost` (`SND_PCM_PLUGIN_DEFINE_FUNC(bifrost)`).
///
/// # Safety
/// Called by alsa-lib with valid pointers.
#[no_mangle]
pub unsafe extern "C" fn _snd_pcm_bifrost_open(pcmp: *mut *mut SndPcm, name: *const c_char, _root: *mut SndConfig, conf: *mut SndConfig, stream: c_int, mode: c_int) -> c_int {
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
        fmt: Fmt::S16,
        evfd,
        shared: None,
        proc_id: None,
        waker: None,
    });
    b.io.version = SND_PCM_IOPLUG_VERSION;
    b.io.name = b.name.as_ptr();
    b.io.flags = SND_PCM_IOPLUG_FLAG_LISTED | SND_PCM_IOPLUG_FLAG_MONOTONIC;
    b.io.poll_fd = evfd;
    b.io.poll_events = if dir == Dir::Playback { POLLOUT as c_uint } else { POLLIN as c_uint };
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
    let err = set_constraints(io, dev_rate);
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
    fn constraint_tables() {
        assert_eq!(FORMATS, [2, 6, 10, 14]);
        assert_eq!(ACCESSES, [0, 3]);
    }
}
