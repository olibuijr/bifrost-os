//! Desktop audio output through the system's libasound (`dlopen`ed, no link-time dependency):
//! the "default" PCM, so PipeWire's or PulseAudio's ALSA plugin does the routing, mixing and
//! sample-rate / format / channel conversion (`snd_pcm_set_params` with soft resampling on).
//! The kernel-ioctl capture path (`alsa`) and the mixer (`mixer`) are separate.

use super::alsa::AudioOut;
use super::dl::Lib;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr, CString};
use std::io;

const STREAM_PLAYBACK: c_int = 0;
const FORMAT_S16_LE: c_uint = 2;
const ACCESS_RW_INTERLEAVED: c_uint = 3;
/// Buffer length asked of the sound server; short enough to keep video in sync, long enough to ride out a scheduling hiccup.
const LATENCY_US: c_uint = 100_000;
/// Buffer length for an exclusive `hw:` device: no mixing server sits in between, so 20 ms is enough
/// and each write reaches the DAC sooner than with dmix's 16 x 1024-frame default.
const LATENCY_HW_US: c_uint = 20_000;
/// Consecutive failed writes (after a recovery attempt each) before the error is reported.
const MAX_FAILURES: u32 = 3;

type Pcm = *mut c_void;

#[derive(Clone, Copy)]
struct Api {
    open: unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int,
    set_params: unsafe extern "C" fn(Pcm, c_uint, c_uint, c_uint, c_uint, c_int, c_uint) -> c_int,
    writei: unsafe extern "C" fn(Pcm, *const c_void, c_ulong) -> c_long,
    recover: unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int,
    delay: unsafe extern "C" fn(Pcm, *mut c_long) -> c_int,
    drop_: unsafe extern "C" fn(Pcm) -> c_int,
    drain: unsafe extern "C" fn(Pcm) -> c_int,
    prepare: unsafe extern "C" fn(Pcm) -> c_int,
    close: unsafe extern "C" fn(Pcm) -> c_int,
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
}

impl Api {
    fn load() -> io::Result<Api> {
        let lib = Lib::open(&["libasound.so.2"])?;
        // SAFETY: each type is the C prototype from <alsa/pcm.h>.
        unsafe {
            Ok(Api {
                open: lib.sym("snd_pcm_open")?,
                set_params: lib.sym("snd_pcm_set_params")?,
                writei: lib.sym("snd_pcm_writei")?,
                recover: lib.sym("snd_pcm_recover")?,
                delay: lib.sym("snd_pcm_delay")?,
                drop_: lib.sym("snd_pcm_drop")?,
                drain: lib.sym("snd_pcm_drain")?,
                prepare: lib.sym("snd_pcm_prepare")?,
                close: lib.sym("snd_pcm_close")?,
                strerror: lib.sym("snd_strerror")?,
            })
        }
    }

    fn error(&self, what: &str, code: c_int) -> io::Error {
        // SAFETY: snd_strerror returns a static NUL-terminated string.
        let msg = unsafe { CStr::from_ptr((self.strerror)(code)) }.to_string_lossy().into_owned();
        io::Error::other(format!("{what}: {msg}"))
    }
}

/// A playback stream on an ALSA PCM of the running sound server.
pub struct AlsaLib {
    api: Api,
    pcm: Pcm,
    channels: usize,
    /// frames accepted since the last `discard`
    written: u64,
}

// SAFETY: the handle is used by one thread at a time (the owner's `&mut self`).
unsafe impl Send for AlsaLib {}

impl AlsaLib {
    /// Opens `SPRINGBOARD_PCM` (a plugin name, not a /dev/snd node) or else "default" for interleaved S16 at `rate`.
    /// `SPRINGBOARD_PCM=hw:0,0` skips dmix and the sound server for a 20 ms buffer (B4); it is exclusive, so
    /// nothing else can play while it is open.
    pub fn open_default(rate: u32, channels: u32) -> io::Result<AlsaLib> {
        let name = std::env::var("SPRINGBOARD_PCM").ok().filter(|s| !s.starts_with("/dev/")).unwrap_or_else(|| "default".into());
        Self::open(&name, rate, channels)
    }

    pub fn open(name: &str, rate: u32, channels: u32) -> io::Result<AlsaLib> {
        let api = Api::load()?;
        let cname = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
        let mut pcm: Pcm = std::ptr::null_mut();
        // SAFETY: out-pointer and name are valid for the call; the handle is closed in Drop.
        let r = unsafe { (api.open)(&mut pcm, cname.as_ptr(), STREAM_PLAYBACK, 0) };
        if r < 0 {
            return Err(api.error(&format!("snd_pcm_open {name}"), r));
        }
        let mut out = AlsaLib { api, pcm, channels: channels as usize, written: 0 };
        let latency = if name.starts_with("hw:") { LATENCY_HW_US } else { LATENCY_US };
        // SAFETY: pcm is open.
        let r = unsafe { (api.set_params)(pcm, FORMAT_S16_LE, ACCESS_RW_INTERLEAVED, channels, rate, 1, latency) };
        if r < 0 {
            let e = api.error("snd_pcm_set_params", r);
            out.close();
            return Err(e);
        }
        Ok(out)
    }

    fn close(&mut self) {
        if !self.pcm.is_null() {
            // SAFETY: the handle is ours and not used after this.
            unsafe { (self.api.close)(self.pcm) };
            self.pcm = std::ptr::null_mut();
        }
    }

    /// Frames still queued in the sound server's buffer.
    fn delay(&self) -> u64 {
        let mut d: c_long = 0;
        // SAFETY: pcm is open and the out-pointer valid.
        let r = unsafe { (self.api.delay)(self.pcm, &mut d) };
        if r < 0 {
            0
        } else {
            d.max(0) as u64
        }
    }
}

impl AudioOut for AlsaLib {
    fn write(&mut self, frames: &[i16]) -> io::Result<()> {
        let total = frames.len() / self.channels.max(1);
        let (mut done, mut failures) = (0usize, 0u32);
        while done < total {
            let rest = &frames[done * self.channels..];
            // SAFETY: `rest` holds (total - done) whole frames; the call blocks until they are queued.
            let r = unsafe { (self.api.writei)(self.pcm, rest.as_ptr() as *const c_void, (total - done) as c_ulong) };
            if r >= 0 {
                done += r as usize;
                self.written += r as u64;
                failures = 0;
                continue;
            }
            // An xrun (EPIPE), a suspend (ESTRPIPE) or a signal (EINTR) is recovered in place and the chunk goes on.
            let code = r as c_int;
            failures += 1;
            // SAFETY: pcm is open.
            let rec = unsafe { (self.api.recover)(self.pcm, code, 1) };
            if rec < 0 || failures >= MAX_FAILURES {
                return Err(self.api.error("snd_pcm_writei", if rec < 0 { rec } else { code }));
            }
        }
        Ok(())
    }

    fn played(&mut self) -> u64 {
        self.written.saturating_sub(self.delay())
    }

    /// Pausing plays out what is queued and stops writing; on resume the player sees `played`
    /// fall back and re-seeks (the same contract as `NullOut::dropping_on_pause`).
    fn pause(&mut self, on: bool) {
        if on {
            self.flush();
        }
    }

    fn flush(&mut self) {
        // SAFETY: pcm is open; drain blocks until the queue has played.
        unsafe {
            (self.api.drain)(self.pcm);
            (self.api.prepare)(self.pcm);
        }
        self.written = 0;
    }

    fn discard(&mut self) {
        // SAFETY: pcm is open.
        unsafe {
            (self.api.drop_)(self.pcm);
            (self.api.prepare)(self.pcm);
        }
        self.written = 0;
    }
}

impl Drop for AlsaLib {
    fn drop(&mut self) {
        if !self.pcm.is_null() {
            // SAFETY: pcm is open; drop discards what is queued, close then releases it.
            unsafe { (self.api.drop_)(self.pcm) };
        }
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_name_with_a_nul_is_refused_without_opening_anything() {
        let e = AlsaLib::open("de\0fault", 48000, 2).err().expect("must fail");
        assert!(matches!(e.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::NotFound));
    }
}
