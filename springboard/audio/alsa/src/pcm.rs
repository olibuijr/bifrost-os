//! One PCM stream: open `/dev/snd/pcmC{c}D{d}{p|c}`, negotiate hw params, set sw params, mmap the data, status and
//! control pages (with a `SYNC_PTR` fallback), read the hardware position without a syscall, commit the
//! application pointer, link streams.
//!
//! hw params (`negotiate`): mmap access (interleaved preferred, then non-interleaved); the format S32_LE, then
//! S24_LE, then S16_LE; the requested rate (else the nearest the device offers); the period size of the graph
//! quantum when the device accepts it; the largest buffer; `NO_PERIOD_WAKEUP` when asked and supported.
//! sw params: timestamps enabled with `MONOTONIC_RAW`, `stop_threshold = boundary` (the kernel never stops the
//! stream on an underrun; the driver detects and recovers it, see `ring`), and no automatic start.

use crate::convert::SampleFormat;
use crate::ring::{ptr_add, unwrap_ptr, RingLayout};
use crate::sys::{close_fd, ioctl, ioctl_int, ioctl_none, open_dev};
use crate::uapi::*;
use std::io;
use std::os::fd::RawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Playback,
    Capture,
}

impl Direction {
    /// The device node suffix: `p` or `c`.
    pub fn suffix(self) -> char {
        match self {
            Direction::Playback => 'p',
            Direction::Capture => 'c',
        }
    }
}

pub fn pcm_path(card: u32, device: u32, dir: Direction) -> String {
    format!("/dev/snd/pcmC{card}D{device}{}", dir.suffix())
}

/// What the driver asks of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmRequest {
    pub rate: u32,
    /// The graph quantum: the period size asked for (frames).
    pub quantum: u32,
    /// Wanted channel count; `None` takes the device's maximum (at most `max_channels`).
    pub channels: Option<u32>,
    pub max_channels: u32,
    /// Ask for `NO_PERIOD_WAKEUP`. The status page then only moves on `hwsync`.
    pub no_period_wakeup: bool,
}

/// What a device offers, from `HW_REFINE` on a fresh parameter set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmCaps {
    pub channels: (u32, u32),
    pub rates: (u32, u32),
    pub buffer_frames: (u32, u32),
    pub formats: Vec<SampleFormat>,
    pub no_period_wakeup: bool,
}

/// `v` limited to `lo..=hi`; a hostile or inverted range (`lo > hi`) gives `lo` instead of panicking like
/// `u32::clamp`.
pub fn clamp_range(v: u32, lo: u32, hi: u32) -> u32 {
    if lo > hi {
        lo
    } else {
        v.clamp(lo, hi)
    }
}

/// Sanity checks on the parameters the kernel fixed, so an odd driver cannot make us map nothing or divide by zero.
pub fn validate_fixed(channels: u32, rate: u32, period: u32, buffer: u32) -> io::Result<()> {
    let bad = |what: &str| Err(io::Error::new(io::ErrorKind::InvalidData, format!("the device fixed {what}")));
    if channels == 0 || channels > 256 {
        return bad("a channel count outside 1 to 256");
    }
    if rate == 0 {
        return bad("a rate of 0");
    }
    if period == 0 || buffer == 0 || buffer < period || buffer > (1 << 24) {
        return bad("an unusable period or buffer size");
    }
    Ok(())
}

fn usable(p: &HwParams) -> bool {
    (PARAM_SAMPLE_BITS..PARAM_SAMPLE_BITS + 12).all(|i| !p.is_empty(i))
}

/// Applies `edit` to a copy of `base` and asks the kernel to refine it; the refined set if it is not empty.
fn attempt<R: FnMut(&mut HwParams) -> io::Result<()>>(refine: &mut R, base: &HwParams, edit: impl FnOnce(&mut HwParams)) -> Option<HwParams> {
    let mut p = *base;
    edit(&mut p);
    refine(&mut p).ok()?;
    usable(&p).then_some(p)
}

/// The hw params for a stream: see the module comment. `refine` is the `HW_REFINE` ioctl (a model in tests).
pub fn negotiate<R: FnMut(&mut HwParams) -> io::Result<()>>(mut refine: R, req: &PcmRequest) -> io::Result<HwParams> {
    let unsupported = |what: &str| io::Error::new(io::ErrorKind::Unsupported, what.to_string());
    let base = HwParams::any();
    // access: interleaved first
    let mut p = attempt(&mut refine, &base, |p| p.set_mask(PARAM_ACCESS, &[ACCESS_MMAP_INTERLEAVED]))
        .or_else(|| attempt(&mut refine, &base, |p| p.set_mask(PARAM_ACCESS, &[ACCESS_MMAP_NONINTERLEAVED])))
        .ok_or_else(|| unsupported("no mmap access"))?;
    // format: the widest the device has
    let cur = p;
    p = SampleFormat::PREFERENCE
        .iter()
        .find_map(|f| attempt(&mut refine, &cur, |p| p.set_mask(PARAM_FORMAT, &[f.alsa()])))
        .ok_or_else(|| unsupported("no S32_LE, S24_LE or S16_LE"))?;
    let cur = p;
    p = attempt(&mut refine, &cur, |p| p.set_mask(PARAM_SUBFORMAT, &[0])).unwrap_or(cur);
    // channels: the request, else the device's maximum up to the cap, else stereo, else its minimum
    let (cmin, cmax) = p.range(PARAM_CHANNELS);
    let want = clamp_range(req.channels.unwrap_or(cmax.min(req.max_channels)), cmin, cmax);
    let cur = p;
    p = [want, 2, cmin]
        .iter()
        .find_map(|&c| attempt(&mut refine, &cur, |p| p.set_range(PARAM_CHANNELS, c, c)))
        .ok_or_else(|| unsupported("no channel count"))?;
    // rate: exact, else the nearest the device offers
    let cur = p;
    p = match attempt(&mut refine, &cur, |p| p.set_range(PARAM_RATE, req.rate, req.rate)) {
        Some(p) => p,
        None => {
            let (rmin, rmax) = cur.range(PARAM_RATE);
            let near = clamp_range(req.rate, rmin, rmax);
            attempt(&mut refine, &cur, |p| p.set_range(PARAM_RATE, near, near)).ok_or_else(|| unsupported("rate"))?
        }
    };
    // period: the quantum when the device takes it
    let cur = p;
    p = attempt(&mut refine, &cur, |p| p.set_range(PARAM_PERIOD_SIZE, req.quantum, req.quantum)).unwrap_or(cur);
    // buffer: the largest the (period-constrained) device offers
    let cur = p;
    let (_, bmax) = cur.range(PARAM_BUFFER_SIZE);
    let step = req.quantum.max(1);
    // whole multiples of the quantum, from the top down
    let top = bmax - bmax % step;
    p = (0..8u32)
        .find_map(|i| {
            let b = u32::try_from((top as u64).checked_sub(i as u64 * step as u64)?).ok().filter(|&b| b > 0)?;
            attempt(&mut refine, &cur, |p| p.set_range(PARAM_BUFFER_SIZE, b, b))
        })
        .unwrap_or(cur);
    if p.info & INFO_MMAP == 0 {
        return Err(unsupported("device cannot mmap"));
    }
    if req.no_period_wakeup && p.info & INFO_NO_PERIOD_WAKEUP != 0 {
        p.flags |= HW_PARAMS_NO_PERIOD_WAKEUP;
    }
    Ok(p)
}

/// The single bit set in a mask parameter.
fn mask_single(p: &HwParams, param: usize) -> Option<u32> {
    let m = &p.masks[param];
    let mut found = None;
    for (w, &word) in m.bits.iter().enumerate() {
        for b in 0..32 {
            if word & (1 << b) != 0 {
                if found.is_some() {
                    return None;
                }
                found = Some(w as u32 * 32 + b);
            }
        }
    }
    found
}

/// A snapshot of the status page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmPos {
    pub state: i32,
    /// Frames since `prepare`, unwrapped from the hardware pointer.
    pub hw_frames: u64,
    /// The hardware pointer as the kernel reports it (modulo the boundary).
    pub hw_raw: u64,
    /// When the pointer was updated (`CLOCK_MONOTONIC_RAW`, ns).
    pub tstamp_ns: u64,
}

pub struct Pcm {
    fd: RawFd,
    pub dir: Direction,
    pub card: u32,
    pub device: u32,
    pub layout: RingLayout,
    pub rate: u32,
    pub channels: u32,
    pub period_frames: u32,
    pub buffer_frames: u32,
    pub boundary: u64,
    pub info: u32,
    pub no_period_wakeup: bool,
    data: *mut u8,
    data_len: usize,
    status: *const MmapStatus,
    control: *mut MmapControl,
    page: usize,
    sync_mode: bool,
    configured: bool,
    /// The application pointer, modulo the boundary (ours: the kernel only reads it).
    appl: u64,
    hw_last_raw: u64,
    hw_total: u64,
}

// SAFETY: the raw pointers are mappings private to this struct, which is used from one thread at a time.
unsafe impl Send for Pcm {}

fn page_size() -> usize {
    // SAFETY: plain query.
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 {
        p as usize
    } else {
        4096
    }
}

impl Pcm {
    /// Opens the node (non-blocking). Fails with `EBUSY` while another client holds the stream.
    pub fn open(card: u32, device: u32, dir: Direction) -> io::Result<Pcm> {
        let fd = open_dev(&pcm_path(card, device, dir), true)?;
        Ok(Pcm {
            fd,
            dir,
            card,
            device,
            layout: RingLayout { interleaved: true, channels: 0, format: SampleFormat::S16, buffer_frames: 1 },
            rate: 0,
            channels: 0,
            period_frames: 0,
            buffer_frames: 0,
            boundary: 1,
            info: 0,
            no_period_wakeup: false,
            data: std::ptr::null_mut(),
            data_len: 0,
            status: std::ptr::null(),
            control: std::ptr::null_mut(),
            page: page_size(),
            sync_mode: false,
            configured: false,
            appl: 0,
            hw_last_raw: 0,
            hw_total: 0,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// What the device offers (before `configure`).
    pub fn caps(&self) -> io::Result<PcmCaps> {
        let mut p = HwParams::any();
        p.set_mask(PARAM_ACCESS, &[ACCESS_MMAP_INTERLEAVED, ACCESS_MMAP_NONINTERLEAVED]);
        // SAFETY: `p` is the struct PCM_HW_REFINE encodes.
        unsafe { ioctl(self.fd, PCM_HW_REFINE, &mut p) }?;
        let formats = SampleFormat::PREFERENCE.iter().copied().filter(|f| p.mask_has(PARAM_FORMAT, f.alsa())).collect();
        Ok(PcmCaps {
            channels: p.range(PARAM_CHANNELS),
            rates: p.range(PARAM_RATE),
            buffer_frames: p.range(PARAM_BUFFER_SIZE),
            formats,
            no_period_wakeup: p.info & INFO_NO_PERIOD_WAKEUP != 0,
        })
    }

    /// Negotiates and installs hw and sw params, maps the pages. The stream ends `SETUP`; call `prepare` next.
    pub fn configure(&mut self, req: &PcmRequest) -> io::Result<()> {
        let fd = self.fd;
        // SAFETY (closure): each `p` is the struct PCM_HW_REFINE encodes.
        let mut hw = negotiate(|p| unsafe { ioctl(fd, PCM_HW_REFINE, p) }, req)?;
        // SAFETY: `hw` is the struct PCM_HW_PARAMS encodes.
        unsafe { ioctl(fd, PCM_HW_PARAMS, &mut hw) }?;
        let format = mask_single(&hw, PARAM_FORMAT).and_then(SampleFormat::from_alsa).ok_or_else(|| io::Error::other("format not fixed"))?;
        let interleaved = mask_single(&hw, PARAM_ACCESS) == Some(ACCESS_MMAP_INTERLEAVED);
        self.channels = hw.range(PARAM_CHANNELS).0;
        self.rate = hw.range(PARAM_RATE).0;
        self.period_frames = hw.range(PARAM_PERIOD_SIZE).0;
        self.buffer_frames = hw.range(PARAM_BUFFER_SIZE).0;
        self.info = hw.info;
        self.no_period_wakeup = hw.flags & HW_PARAMS_NO_PERIOD_WAKEUP != 0;
        validate_fixed(self.channels, self.rate, self.period_frames, self.buffer_frames)?;
        self.layout = RingLayout { interleaved, channels: self.channels as usize, format, buffer_frames: self.buffer_frames as usize };
        self.set_sw_params()?;
        self.map()?;
        self.configured = true;
        Ok(())
    }

    fn set_sw_params(&mut self) -> io::Result<()> {
        let mut sw = SwParams::zeroed();
        sw.tstamp_mode = TSTAMP_MODE_ENABLE;
        sw.tstamp_type = TSTAMP_TYPE_MONOTONIC_RAW;
        sw.period_step = 1;
        sw.avail_min = self.period_frames.max(1) as u64;
        sw.start_threshold = 1 << 32;
        sw.stop_threshold = 1 << 32;
        // SAFETY: `sw` is the struct PCM_SW_PARAMS encodes.
        unsafe { ioctl(self.fd, PCM_SW_PARAMS, &mut sw) }?;
        // the kernel answers with the boundary; the stream stops only there, and starts only on START
        self.boundary = sw.boundary.max(self.buffer_frames as u64).max(1);
        sw.stop_threshold = self.boundary;
        sw.start_threshold = self.boundary;
        // SAFETY: as above.
        unsafe { ioctl(self.fd, PCM_SW_PARAMS, &mut sw) }
    }

    fn map_at(&self, len: usize, prot: libc::c_int, offset: i64) -> Option<*mut u8> {
        // SAFETY: a shared mapping of the PCM fd; failure is MAP_FAILED.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, prot, libc::MAP_SHARED, self.fd, offset as libc::off_t) };
        (p != libc::MAP_FAILED).then_some(p as *mut u8)
    }

    fn map(&mut self) -> io::Result<()> {
        let len = self.layout.ring_bytes();
        let prot = match self.dir {
            Direction::Playback => libc::PROT_READ | libc::PROT_WRITE,
            Direction::Capture => libc::PROT_READ,
        };
        self.data = self.map_at(len, prot, MMAP_OFFSET_DATA).ok_or_else(io::Error::last_os_error)?;
        self.data_len = len;
        self.sync_mode = self.info & INFO_SYNC_APPLPTR != 0;
        if !self.sync_mode {
            let st = self.map_at(self.page, libc::PROT_READ, MMAP_OFFSET_STATUS_NEW).or_else(|| self.map_at(self.page, libc::PROT_READ, MMAP_OFFSET_STATUS_OLD));
            let ct = self.map_at(self.page, libc::PROT_READ | libc::PROT_WRITE, MMAP_OFFSET_CONTROL_NEW).or_else(|| self.map_at(self.page, libc::PROT_READ | libc::PROT_WRITE, MMAP_OFFSET_CONTROL_OLD));
            match (st, ct) {
                (Some(s), Some(c)) => {
                    self.status = s as *const MmapStatus;
                    self.control = c as *mut MmapControl;
                }
                (s, c) => {
                    // SAFETY: unmapping what was just mapped, once.
                    unsafe {
                        if let Some(s) = s {
                            libc::munmap(s as *mut libc::c_void, self.page);
                        }
                        if let Some(c) = c {
                            libc::munmap(c as *mut libc::c_void, self.page);
                        }
                    }
                    self.sync_mode = true;
                    eprintln!("[audio/driver] pcm {}: status/control mmap refused, using SYNC_PTR", pcm_path(self.card, self.device, self.dir));
                }
            }
        }
        Ok(())
    }

    /// True when the pointers go through `SYNC_PTR` calls instead of the mapped pages.
    pub fn uses_sync_ptr(&self) -> bool {
        self.sync_mode
    }

    /// The ring as bytes. The kernel's DMA engine reads or writes the same memory; the driver only touches the
    /// frames between the hardware and application pointers it computed.
    pub fn ring_mut(&mut self) -> &mut [u8] {
        if self.data.is_null() {
            return &mut [];
        }
        // SAFETY: `data` maps `data_len` bytes for the life of `self`; playback maps it writable. A capture ring is
        // mapped read-only and is only ever read through `ring`.
        unsafe { std::slice::from_raw_parts_mut(self.data, self.data_len) }
    }

    pub fn ring(&self) -> &[u8] {
        if self.data.is_null() {
            return &[];
        }
        // SAFETY: as in `ring_mut`.
        unsafe { std::slice::from_raw_parts(self.data, self.data_len) }
    }

    /// `PREPARE`: resets both pointers to zero.
    pub fn prepare(&mut self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_PREPARE)?;
        self.appl = 0;
        self.hw_last_raw = 0;
        self.hw_total = 0;
        Ok(())
    }

    /// `RESUME` after a system suspend: `Ok` when the stream is running again, `EAGAIN` while the hardware is
    /// not ready, `ENOSYS` when the driver cannot resume (then `prepare` and `start` again).
    pub fn resume(&self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_RESUME)
    }

    pub fn start(&self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_START)
    }

    /// Stops the stream at once and discards queued frames.
    pub fn drop_stream(&self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_DROP)
    }

    /// Links `other` to this stream so that `start` starts both at the same hardware tick.
    pub fn link(&self, other: &Pcm) -> io::Result<()> {
        ioctl_int(self.fd, PCM_LINK, other.fd)
    }

    pub fn unlink(&self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_UNLINK)
    }

    /// Asks the driver to read the hardware pointer now (needed with `no_period_wakeup`).
    pub fn hwsync(&self) -> io::Result<()> {
        ioctl_none(self.fd, PCM_HWSYNC)
    }

    fn sync_ptr(&mut self, flags: u32, appl: Option<u64>) -> io::Result<SyncPtr> {
        let mut sp = SyncPtr::zeroed();
        sp.flags = flags;
        if let Some(a) = appl {
            sp.control.appl_ptr = a;
        }
        // SAFETY: `sp` is the struct PCM_SYNC_PTR encodes.
        unsafe { ioctl(self.fd, PCM_SYNC_PTR, &mut sp) }?;
        Ok(sp)
    }

    /// Reads state, hardware pointer and its timestamp. No syscall when the status page is mapped.
    pub fn status(&mut self) -> PcmPos {
        let (state, hw_raw, ts) = if !self.status.is_null() && !self.sync_mode {
            self.read_mapped()
        } else {
            match self.sync_ptr(if self.no_period_wakeup { SYNC_PTR_HWSYNC } else { 0 }, None) {
                Ok(sp) => (sp.status.state, sp.status.hw_ptr, sp.status.tstamp.ns()),
                Err(_) => (STATE_DISCONNECTED, self.hw_last_raw, 0),
            }
        };
        let hw_raw = hw_raw % self.boundary.max(1);
        self.hw_total = unwrap_ptr(self.hw_last_raw, self.hw_total, hw_raw, self.boundary.max(1));
        self.hw_last_raw = hw_raw;
        PcmPos { state, hw_frames: self.hw_total, hw_raw, tstamp_ns: ts }
    }

    /// Two equal consecutive reads of the pair (pointer, timestamp): the kernel writes them without a lock.
    fn read_mapped(&self) -> (i32, u64, u64) {
        let st = self.status;
        let read = || {
            // SAFETY: `status` maps a whole MmapStatus for the life of `self`; the kernel updates it concurrently,
            // so the fields are read volatile.
            unsafe {
                let ts = std::ptr::read_volatile(std::ptr::addr_of!((*st).tstamp));
                let hw = std::ptr::read_volatile(std::ptr::addr_of!((*st).hw_ptr));
                let state = std::ptr::read_volatile(std::ptr::addr_of!((*st).state));
                (state, hw, ts)
            }
        };
        let mut a = read();
        for _ in 0..8 {
            let b = read();
            if a.1 == b.1 && a.2 == b.2 {
                break;
            }
            a = b;
        }
        (a.0, a.1, a.2.ns())
    }

    /// The application pointer (frames written or read, modulo the boundary).
    pub fn appl(&self) -> u64 {
        self.appl
    }

    /// Sets the application pointer to `appl` and tells the kernel (through the control page, no syscall, or
    /// `SYNC_PTR`).
    pub fn commit_appl(&mut self, appl: u64) -> io::Result<()> {
        self.appl = appl % self.boundary.max(1);
        if !self.control.is_null() && !self.sync_mode {
            // SAFETY: `control` maps a whole MmapControl, writable, for the life of `self`.
            unsafe { std::ptr::write_volatile(std::ptr::addr_of_mut!((*self.control).appl_ptr), self.appl) };
            std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
            Ok(())
        } else {
            self.sync_ptr(SYNC_PTR_APPL, Some(self.appl)).map(|_| ())
        }
    }

    /// Sets the application pointer to `from + frames` and commits it.
    pub fn advance_to(&mut self, from: u64, frames: u64) -> io::Result<()> {
        let to = ptr_add(from, frames, self.boundary.max(1));
        self.commit_appl(to)
    }

    /// Advances the application pointer by `frames` and commits it.
    pub fn advance(&mut self, frames: u64) -> io::Result<()> {
        let to = ptr_add(self.appl, frames, self.boundary.max(1));
        self.commit_appl(to)
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        // SAFETY: the mappings and the fd belong to `self` and are released once.
        unsafe {
            if !self.data.is_null() {
                libc::munmap(self.data as *mut libc::c_void, self.data_len);
            }
            if !self.status.is_null() {
                libc::munmap(self.status as *mut libc::c_void, self.page);
            }
            if !self.control.is_null() {
                libc::munmap(self.control as *mut libc::c_void, self.page);
            }
        }
        if self.configured {
            let _ = ioctl_none(self.fd, PCM_DROP);
        }
        close_fd(self.fd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model of what a kernel PCM device offers: `HW_REFINE` intersects the request with it.
    struct Device {
        access: Vec<u32>,
        formats: Vec<u32>,
        channels: (u32, u32),
        rates: (u32, u32),
        /// Period sizes that work.
        periods: Vec<u32>,
        buffer_max: u32,
        no_wakeup: bool,
    }

    fn intersect(p: &mut HwParams, param: usize, lo: u32, hi: u32) -> io::Result<()> {
        let (a, b) = p.range(param);
        let (a, b) = (a.max(lo), b.min(hi));
        if a > b {
            p.intervals[param - 8].flags |= INTERVAL_EMPTY;
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        p.set_range(param, a, b);
        Ok(())
    }

    fn narrow_mask(p: &mut HwParams, param: usize, allowed: &[u32]) -> io::Result<()> {
        let keep: Vec<u32> = (0..256).filter(|&b| p.mask_has(param, b) && allowed.contains(&b)).collect();
        if keep.is_empty() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        p.set_mask(param, &keep);
        Ok(())
    }

    impl Device {
        fn refine(&self, p: &mut HwParams) -> io::Result<()> {
            narrow_mask(p, PARAM_ACCESS, &self.access)?;
            narrow_mask(p, PARAM_FORMAT, &self.formats)?;
            intersect(p, PARAM_CHANNELS, self.channels.0, self.channels.1)?;
            intersect(p, PARAM_RATE, self.rates.0, self.rates.1)?;
            let (pmin, pmax) = (*self.periods.iter().min().unwrap(), *self.periods.iter().max().unwrap());
            intersect(p, PARAM_PERIOD_SIZE, pmin, pmax)?;
            let (a, b) = p.range(PARAM_PERIOD_SIZE);
            if a == b && !self.periods.contains(&a) {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            // buffers are whole multiples of the period (at least two)
            let period = p.range(PARAM_PERIOD_SIZE).0;
            intersect(p, PARAM_BUFFER_SIZE, period * 2, self.buffer_max)?;
            let (bl, bh) = p.range(PARAM_BUFFER_SIZE);
            if bl == bh && bl % period != 0 {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            p.info = INFO_MMAP | INFO_MMAP_VALID | if self.no_wakeup { INFO_NO_PERIOD_WAKEUP } else { 0 };
            Ok(())
        }
    }

    fn hda() -> Device {
        Device {
            access: vec![ACCESS_MMAP_INTERLEAVED, ACCESS_MMAP_NONINTERLEAVED],
            formats: vec![FORMAT_S16_LE, FORMAT_S24_LE, FORMAT_S32_LE],
            channels: (2, 8),
            rates: (44_100, 192_000),
            periods: vec![32, 64, 128, 256, 512],
            buffer_max: 2048,
            no_wakeup: true,
        }
    }

    fn req(rate: u32, quantum: u32) -> PcmRequest {
        PcmRequest { rate, quantum, channels: None, max_channels: 2, no_period_wakeup: false }
    }

    fn run(dev: &Device, r: &PcmRequest) -> io::Result<HwParams> {
        negotiate(|p| dev.refine(p), r)
    }

    #[test]
    fn prefers_interleaved_s32_native_rate_quantum_period_and_largest_buffer() {
        let p = run(&hda(), &req(48_000, 64)).unwrap();
        assert_eq!(mask_single(&p, PARAM_ACCESS), Some(ACCESS_MMAP_INTERLEAVED));
        assert_eq!(mask_single(&p, PARAM_FORMAT), Some(FORMAT_S32_LE));
        assert_eq!(p.range(PARAM_RATE), (48_000, 48_000));
        assert_eq!(p.range(PARAM_PERIOD_SIZE), (64, 64));
        // largest multiple of 64 at most 2048
        assert_eq!(p.range(PARAM_BUFFER_SIZE), (2048, 2048));
        // the channel cap of 2 applies to an 8-channel device
        assert_eq!(p.range(PARAM_CHANNELS), (2, 2));
        assert_eq!(p.flags & HW_PARAMS_NO_PERIOD_WAKEUP, 0, "wakeups stay on unless asked");
    }

    #[test]
    fn falls_back_to_s24_then_s16() {
        let mut d = hda();
        d.formats = vec![FORMAT_S16_LE, FORMAT_S24_LE];
        assert_eq!(mask_single(&run(&d, &req(48_000, 64)).unwrap(), PARAM_FORMAT), Some(FORMAT_S24_LE));
        d.formats = vec![FORMAT_S16_LE];
        assert_eq!(mask_single(&run(&d, &req(48_000, 64)).unwrap(), PARAM_FORMAT), Some(FORMAT_S16_LE));
        d.formats = vec![0]; // U8 only
        assert!(run(&d, &req(48_000, 64)).is_err());
    }

    #[test]
    fn non_interleaved_only_devices_are_accepted() {
        let mut d = hda();
        d.access = vec![ACCESS_MMAP_NONINTERLEAVED];
        assert_eq!(mask_single(&run(&d, &req(48_000, 64)).unwrap(), PARAM_ACCESS), Some(ACCESS_MMAP_NONINTERLEAVED));
        d.access = vec![3]; // read/write only
        assert!(run(&d, &req(48_000, 64)).is_err());
    }

    #[test]
    fn an_unsupported_rate_takes_the_nearest_the_device_offers() {
        let mut d = hda();
        d.rates = (48_000, 48_000);
        assert_eq!(run(&d, &req(96_000, 64)).unwrap().range(PARAM_RATE), (48_000, 48_000));
        assert_eq!(run(&d, &req(44_100, 64)).unwrap().range(PARAM_RATE), (48_000, 48_000));
    }

    #[test]
    fn an_unsupported_period_leaves_it_to_the_kernel() {
        let p = run(&hda(), &req(48_000, 48)).unwrap();
        let (lo, hi) = p.range(PARAM_PERIOD_SIZE);
        assert!(lo < hi, "period stays a range: {lo}..{hi}");
        // the buffer is still the largest
        assert!(p.range(PARAM_BUFFER_SIZE).0 > 1024);
    }

    #[test]
    fn the_buffer_steps_down_until_the_device_accepts_it() {
        let mut d = hda();
        d.buffer_max = 2000; // not a multiple of 64: 1984 is the largest that is
        assert_eq!(run(&d, &req(48_000, 64)).unwrap().range(PARAM_BUFFER_SIZE), (1984, 1984));
    }

    #[test]
    fn no_period_wakeup_needs_both_the_request_and_the_device() {
        let mut r = req(48_000, 64);
        r.no_period_wakeup = true;
        assert_ne!(run(&hda(), &r).unwrap().flags & HW_PARAMS_NO_PERIOD_WAKEUP, 0);
        let mut d = hda();
        d.no_wakeup = false;
        assert_eq!(run(&d, &r).unwrap().flags & HW_PARAMS_NO_PERIOD_WAKEUP, 0);
    }

    #[test]
    fn channel_choice_follows_request_cap_and_device() {
        let mut r = req(48_000, 64);
        r.max_channels = 8;
        assert_eq!(run(&hda(), &r).unwrap().range(PARAM_CHANNELS), (8, 8), "no request: the device maximum up to the cap");
        r.channels = Some(4);
        assert_eq!(run(&hda(), &r).unwrap().range(PARAM_CHANNELS), (4, 4));
        r.channels = Some(16);
        assert_eq!(run(&hda(), &r).unwrap().range(PARAM_CHANNELS), (8, 8), "clamped to what the device has");
        let mut d = hda();
        d.channels = (1, 1);
        r.channels = None;
        assert_eq!(run(&d, &r).unwrap().range(PARAM_CHANNELS), (1, 1));
    }

    #[test]
    fn a_device_without_mmap_is_refused() {
        let err = negotiate(
            |q| {
                q.info = 0;
                Ok(())
            },
            &req(48_000, 64),
        );
        assert_eq!(err.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn mask_single_finds_the_one_bit_or_none() {
        let mut p = HwParams::any();
        assert_eq!(mask_single(&p, PARAM_FORMAT), None);
        p.set_mask(PARAM_FORMAT, &[FORMAT_S32_LE]);
        assert_eq!(mask_single(&p, PARAM_FORMAT), Some(10));
        p.set_mask(PARAM_FORMAT, &[35]);
        assert_eq!(mask_single(&p, PARAM_FORMAT), Some(35));
    }

    #[test]
    fn hostile_ranges_do_not_panic() {
        assert_eq!(clamp_range(5, 8, 2), 8);
        assert_eq!(clamp_range(5, 2, 8), 5);
        assert_eq!(clamp_range(1, 2, 8), 2);
        assert_eq!(clamp_range(9, 2, 8), 8);
        // A kernel that answers inverted channel and rate ranges, a huge buffer maximum and odd quanta.
        for quantum in [0, 1, 64, u32::MAX] {
            let r = negotiate(
                |p| {
                    p.set_range(PARAM_CHANNELS, 8, 2);
                    p.set_range(PARAM_RATE, 48_000, 44_100);
                    p.set_range(PARAM_BUFFER_SIZE, 1, u32::MAX);
                    p.info = INFO_MMAP;
                    Ok(())
                },
                &req(48_000, quantum),
            );
            let _ = r;
        }
    }

    #[test]
    fn fixed_parameters_are_validated() {
        assert!(validate_fixed(2, 48_000, 64, 2048).is_ok());
        assert!(validate_fixed(0, 48_000, 64, 2048).is_err());
        assert!(validate_fixed(257, 48_000, 64, 2048).is_err());
        assert!(validate_fixed(2, 0, 64, 2048).is_err());
        assert!(validate_fixed(2, 48_000, 0, 2048).is_err());
        assert!(validate_fixed(2, 48_000, 64, 0).is_err());
        assert!(validate_fixed(2, 48_000, 4096, 2048).is_err(), "a buffer smaller than a period");
        assert!(validate_fixed(2, 48_000, 64, (1 << 24) + 1).is_err());
        assert!(validate_fixed(2, 48_000, 64, 1 << 24).is_ok());
    }

    #[test]
    fn the_buffer_search_survives_a_huge_quantum() {
        // quantum * 7 overflows u32; the search must stop at zero instead of panicking.
        let mut d = hda();
        d.buffer_max = u32::MAX;
        let r = run(&d, &req(48_000, 1 << 30));
        let _ = r;
    }

    #[test]
    fn device_node_paths() {
        assert_eq!(pcm_path(0, 0, Direction::Playback), "/dev/snd/pcmC0D0p");
        assert_eq!(pcm_path(1, 3, Direction::Capture), "/dev/snd/pcmC1D3c");
    }
}
