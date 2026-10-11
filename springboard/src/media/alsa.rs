//! ALSA without alsa-lib: PCM capture through the kernel's ioctl interface on /dev/snd, plus
//! the `AudioOut` trait the players write to. Playback goes through libasound (`super::alsalib`);
//! the mixer side (controls) is `super::mixer`. The uapi struct layouts are pinned by tests the
//! same way kms.rs pins the DRM ones.

use crate::bare::sys::{close, io, ioc, ioctl_ptr, iowr, open_dev, O_NONBLOCK, O_RDWR};
use std::ffi::{c_int, c_void};
use std::io;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Interval {
    min: u32,
    max: u32,
    /// bit 0 openmin, 1 openmax, 2 integer, 3 empty
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Mask {
    bits: [u32; 8],
}

/// struct snd_pcm_hw_params
#[repr(C)]
struct HwParams {
    flags: u32,
    masks: [Mask; 3],
    mres: [Mask; 5],
    intervals: [Interval; 12],
    ires: [Interval; 9],
    rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    fifo_size: u64,
    reserved: [u8; 64],
}

/// struct snd_pcm_sw_params
#[repr(C)]
struct SwParams {
    tstamp_mode: i32,
    period_step: u32,
    sleep_min: u32,
    avail_min: u64,
    xfer_align: u64,
    start_threshold: u64,
    stop_threshold: u64,
    silence_threshold: u64,
    silence_size: u64,
    boundary: u64,
    proto: u32,
    tstamp_type: u32,
    reserved: [u8; 56],
}

/// struct snd_xferi
#[repr(C)]
struct Xferi {
    result: i64,
    buf: *mut c_void,
    frames: u64,
}

const PARAM_ACCESS: usize = 0;
const PARAM_FORMAT: usize = 1;
const PARAM_SUBFORMAT: usize = 2;
/// interval params are numbered from 8
const PARAM_SAMPLE_BITS: usize = 8;
const PARAM_FRAME_BITS: usize = 9;
const PARAM_CHANNELS: usize = 10;
const PARAM_RATE: usize = 11;
const PARAM_PERIOD_SIZE: usize = 13;
const PARAM_PERIODS: usize = 15;
const PARAM_BUFFER_SIZE: usize = 17;
const ACCESS_RW_INTERLEAVED: u32 = 3;
const FORMAT_S16_LE: u32 = 2;

fn ior<T>(ty: u8, nr: u8) -> u32 {
    ioc(2, ty, nr, std::mem::size_of::<T>())
}
fn pcm_hw_params() -> u32 {
    iowr::<HwParams>(b'A', 0x11)
}
fn pcm_sw_params() -> u32 {
    iowr::<SwParams>(b'A', 0x13)
}
fn pcm_prepare() -> u32 {
    io(b'A', 0x40)
}
fn pcm_drop() -> u32 {
    io(b'A', 0x43)
}

impl HwParams {
    /// Everything allowed, as alsa-lib's snd_pcm_hw_params_any.
    fn any() -> Self {
        let full = Interval { min: 0, max: u32::MAX, flags: 0 };
        let mut p = HwParams {
            flags: 0,
            masks: [Mask { bits: [u32::MAX; 8] }; 3],
            mres: [Mask::default(); 5],
            intervals: [full; 12],
            ires: [Interval::default(); 9],
            rmask: u32::MAX,
            cmask: 0,
            info: 0,
            msbits: 0,
            rate_num: 0,
            rate_den: 0,
            fifo_size: 0,
            reserved: [0; 64],
        };
        for i in [PARAM_SAMPLE_BITS, PARAM_FRAME_BITS, PARAM_CHANNELS, PARAM_RATE, PARAM_PERIOD_SIZE, PARAM_PERIODS, PARAM_BUFFER_SIZE] {
            p.intervals[i - 8].flags = 4; // integer
        }
        p
    }

    fn set_mask(&mut self, param: usize, bit: u32) {
        self.masks[param] = Mask::default();
        self.masks[param].bits[(bit / 32) as usize] = 1 << (bit % 32);
    }

    fn set_range(&mut self, param: usize, min: u32, max: u32) {
        let i = &mut self.intervals[param - 8];
        i.min = min;
        i.max = max;
    }

    fn get(&self, param: usize) -> (u32, u32) {
        let i = &self.intervals[param - 8];
        (i.min, i.max)
    }

}

/// Something that consumes 48 kHz stereo S16 and reports how far it got;
/// the player's clock is derived from `played`.
pub trait AudioOut: Send {
    /// Blocks until the interleaved frames are queued.
    fn write(&mut self, frames: &[i16]) -> io::Result<()>;
    /// Frames that have reached the speaker since the last `discard`.
    fn played(&mut self) -> u64;
    fn pause(&mut self, on: bool);
    /// Lets what is queued play out over silence, then drops it: no cut mid-wave (a click) and
    /// nothing stale left. Defaults to a plain `discard`.
    fn flush(&mut self) {
        self.discard();
    }
    /// Drops everything queued and restarts the count.
    fn discard(&mut self);
}

/// Waits between retries of a failed write; one more attempt than there are entries.
pub const WRITE_RETRY_BACKOFF: [std::time::Duration; 3] = [std::time::Duration::from_millis(20), std::time::Duration::from_millis(60), std::time::Duration::from_millis(180)];

/// Writes the chunk, retrying the same chunk after each `backoff` wait; the last error when all
/// attempts fail, so a hiccup does not skip audio and a dead device is reported.
pub fn write_with_retry(out: &mut dyn AudioOut, frames: &[i16], backoff: &[std::time::Duration]) -> io::Result<()> {
    let mut waits = backoff.iter();
    loop {
        match out.write(frames) {
            Ok(()) => return Ok(()),
            Err(e) => match waits.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => return Err(e),
            },
        }
    }
}

const EAGAIN: i32 = 11;
/// An opened capture stream for `super::capture`: the fd plus the geometry the driver granted.
pub(super) struct CaptureFd {
    pub fd: c_int,
    pub period: u32,
}

fn pcm_start() -> u32 {
    io(b'A', 0x42)
}
fn pcm_readi() -> u32 {
    ior::<Xferi>(b'A', 0x51)
}

/// Opens a capture PCM (e.g. /dev/snd/pcmC0D2c) for interleaved S16 at `rate`/`channels` and
/// starts it. Errors when the device refuses that combination, so the caller can try the next.
pub(super) fn open_capture(path: &str, rate: u32, channels: u32) -> io::Result<CaptureFd> {
    let fd = open_dev(path, O_RDWR | O_NONBLOCK)?;
    let mut hw = HwParams::any();
    hw.set_mask(PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
    hw.set_mask(PARAM_FORMAT, FORMAT_S16_LE);
    hw.set_mask(PARAM_SUBFORMAT, 0);
    hw.set_range(PARAM_SAMPLE_BITS, 16, 16);
    hw.set_range(PARAM_FRAME_BITS, 16 * channels, 16 * channels);
    hw.set_range(PARAM_CHANNELS, channels, channels);
    hw.set_range(PARAM_RATE, rate, rate);
    let setup = (|| -> io::Result<u32> {
        unsafe { ioctl_ptr(fd, pcm_hw_params(), &mut hw) }?;
        let period = hw.get(PARAM_PERIOD_SIZE).0.max(1);
        let buffer = hw.get(PARAM_BUFFER_SIZE).0.max(period);
        let mut sw = SwParams {
            tstamp_mode: 0,
            period_step: 1,
            sleep_min: 0,
            avail_min: period as u64,
            xfer_align: 1,
            start_threshold: 1,
            stop_threshold: (buffer as u64) << 20,
            silence_threshold: 0,
            silence_size: 0,
            boundary: (buffer as u64) << 20,
            proto: 0,
            tstamp_type: 0,
            reserved: [0; 56],
        };
        unsafe { ioctl_ptr(fd, pcm_sw_params(), &mut sw) }?;
        unsafe { ioctl_ptr::<()>(fd, pcm_prepare(), std::ptr::null_mut()) }?;
        unsafe { ioctl_ptr::<()>(fd, pcm_start(), std::ptr::null_mut()) }?;
        Ok(period)
    })();
    match setup {
        Ok(period) => Ok(CaptureFd { fd, period }),
        Err(e) => {
            // SAFETY: fd came from open_dev and is closed exactly once.
            unsafe { close(fd) };
            Err(e)
        }
    }
}

/// Reads up to `buf.len() / channels` frames; Ok(0) when nothing is ready yet (EAGAIN). An
/// overrun (EPIPE) re-prepares and restarts the stream.
pub(super) fn read_capture(fd: c_int, channels: u32, buf: &mut [i16]) -> io::Result<usize> {
    let per_frame = channels as usize;
    let mut x = Xferi { result: 0, buf: buf.as_mut_ptr() as *mut c_void, frames: (buf.len() / per_frame) as u64 };
    match unsafe { ioctl_ptr(fd, pcm_readi(), &mut x) } {
        Ok(()) => Ok(x.result.max(0) as usize),
        Err(e) if e.raw_os_error() == Some(EAGAIN) => Ok(0),
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(86)) => {
            unsafe { ioctl_ptr::<()>(fd, pcm_prepare(), std::ptr::null_mut()) }?;
            unsafe { ioctl_ptr::<()>(fd, pcm_start(), std::ptr::null_mut()) }?;
            Ok(0)
        }
        Err(e) => Err(e),
    }
}

/// Stops and closes a capture stream.
pub(super) fn close_capture(fd: c_int) {
    let _ = unsafe { ioctl_ptr::<()>(fd, pcm_drop(), std::ptr::null_mut()) };
    // SAFETY: the fd is the caller's and not used after this.
    unsafe { close(fd) };
}

/// A silent output that advances at wall-clock speed; used when no PCM
/// device opens (and by host tests) so video still plays.
pub struct NullOut {
    start: std::time::Instant,
    /// frames played before the current run (while paused this is all)
    base: u64,
    paused: bool,
    written: u64,
    /// Behaves like a PCM that drops its queue on pause: pausing drops the queue and `played` restarts at 0.
    drops_on_pause: bool,
}

impl NullOut {
    pub fn new() -> Self {
        NullOut { start: std::time::Instant::now(), base: 0, paused: false, written: 0, drops_on_pause: false }
    }

    /// A fake that, like some PCM drivers, loses its position when paused.
    pub fn dropping_on_pause() -> Self {
        NullOut { drops_on_pause: true, ..Self::new() }
    }
}

impl Default for NullOut {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOut for NullOut {
    fn write(&mut self, frames: &[i16]) -> io::Result<()> {
        self.written += (frames.len() / 2) as u64;
        // keep about 100 ms queued, like a real device would
        while !self.paused && self.written > self.played() + 4800 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }

    fn played(&mut self) -> u64 {
        let run = if self.paused { 0 } else { (self.start.elapsed().as_secs_f64() * 48_000.0) as u64 };
        (self.base + run).min(self.written)
    }

    fn pause(&mut self, on: bool) {
        if on != self.paused {
            self.base = self.played();
            self.start = std::time::Instant::now();
            self.paused = on;
            if on && self.drops_on_pause {
                self.discard();
            }
        }
    }

    fn discard(&mut self) {
        self.base = 0;
        self.written = 0;
        self.start = std::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uapi_struct_sizes() {
        use std::mem::size_of;
        assert_eq!(size_of::<Interval>(), 12);
        assert_eq!(size_of::<Mask>(), 32);
        assert_eq!(size_of::<HwParams>(), 608);
        assert_eq!(size_of::<SwParams>(), 136);
        assert_eq!(size_of::<Xferi>(), 24);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        assert_eq!(pcm_hw_params(), 0xc260_4111);
        assert_eq!(pcm_sw_params(), 0xc088_4113);
        assert_eq!(pcm_prepare(), 0x4140);
        assert_eq!(pcm_drop(), 0x4143);
    }

    #[test]
    fn null_output_paces_and_pauses() {
        let mut o = NullOut::new();
        o.write(&[0; 2 * 480]).unwrap();
        assert!(o.played() <= 480);
        o.pause(true);
        let p = o.played();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(o.played(), p, "frozen while paused");
        o.pause(false);
        o.write(&[0; 2 * 9600]).unwrap();
        assert!(o.played() > 4000, "write blocks until the queue drains to ~100 ms");
        o.discard();
        assert_eq!(o.played(), 0);
    }

    #[test]
    fn dropping_null_output_restarts_at_zero_after_pause() {
        let mut o = NullOut::dropping_on_pause();
        o.write(&[0; 2 * 480]).unwrap();
        o.pause(true);
        o.pause(false);
        assert_eq!(o.played(), 0);
    }

    /// Fails its first `fails` writes.
    struct Flaky {
        fails: usize,
        attempts: usize,
        chunks: Vec<usize>,
    }

    impl AudioOut for Flaky {
        fn write(&mut self, frames: &[i16]) -> io::Result<()> {
            self.attempts += 1;
            if self.fails > 0 {
                self.fails -= 1;
                return Err(io::Error::other("busy"));
            }
            self.chunks.push(frames.len());
            Ok(())
        }
        fn played(&mut self) -> u64 {
            0
        }
        fn pause(&mut self, _: bool) {}
        fn discard(&mut self) {}
    }

    #[test]
    fn a_failed_write_is_retried_with_the_same_chunk_then_reported() {
        let none = [std::time::Duration::ZERO; 3];
        let mut o = Flaky { fails: 2, attempts: 0, chunks: vec![] };
        write_with_retry(&mut o, &[0; 8], &none).unwrap();
        assert_eq!((o.attempts, o.chunks.clone()), (3, vec![8]));
        let mut dead = Flaky { fails: 99, attempts: 0, chunks: vec![] };
        assert!(write_with_retry(&mut dead, &[0; 8], &none).is_err());
        assert_eq!(dead.attempts, 4, "one try plus one per backoff");
    }

}
