//! Speaker output on the desktop: an IO procedure on the default output device of `bifrost-audiod`
//! (the HAL client library), fed from a lock-free ring. No libasound, no `/dev/snd` here. The
//! stream opens on the first write and is released again after `IDLE_CLOSE` without playback so
//! a paused or finished player does not keep the device running (opening the Music app alone
//! never does). System sounds use the sound-effects role (`SpeakerOut::system_sounds`).
//!
//! The IO procedure never allocates, locks or makes a syscall: it reads the ring (atomics only),
//! resamples linearly when the device rate differs from the source rate, and writes the
//! buffers of the cycle.

use super::audio_out::AudioOut;
use bifrost_audio_hal::{Device, IoContext, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::{props, AudioError, BufferList, PropertyAddress, PropertyValue, TimeStamp};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The client name the media player says hello with.
pub const CLIENT: &str = "is.olibuijr.mediaplayer";

/// Playback idle for this long closes the stream.
const IDLE_CLOSE: Duration = Duration::from_secs(5);

const REPORT_EVERY: Duration = Duration::from_secs(1);

/// Ring capacity in frames: about 0.5 s at 48 kHz, enough to ride out a decoder hiccup.
const RING_FRAMES: usize = 24_576;

/// A write waits at most this long for room before it reports a stalled device.
const WRITE_STALL: Duration = Duration::from_secs(3);

/// What was handed to the ring since the last report: the trace that tells silence from stalls.
#[derive(Default, Debug, PartialEq)]
struct Meter {
    frames: u64,
    peak: u16,
    writes: u32,
    /// the longest single write, i.e. how long the writer waited for room
    slowest: Duration,
}

impl Meter {
    fn add(&mut self, samples: &[i16], took: Duration) {
        self.frames += samples.len() as u64 / 2;
        self.peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0).max(self.peak);
        self.writes += 1;
        self.slowest = self.slowest.max(took);
    }

    fn take(&mut self) -> Meter {
        std::mem::take(self)
    }
}

/// Single-producer single-consumer ring of stereo frames (f32 bits in atomics, so no `unsafe`).
/// The writer is the player thread, the reader the IO procedure.
pub struct Ring {
    cells: Box<[AtomicU32]>,
    /// frames written, ever
    head: AtomicU64,
    /// frames read or skipped, ever
    tail: AtomicU64,
    /// the reader skips ahead to this head value (discard)
    clear_upto: AtomicU64,
    /// `head` at the last discard: `played` counts from here
    base: AtomicU64,
    paused: AtomicBool,
    underruns: AtomicU32,
}

impl Ring {
    pub fn new(frames: usize) -> Arc<Ring> {
        Arc::new(Ring {
            cells: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            clear_upto: AtomicU64::new(0),
            base: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            underruns: AtomicU32::new(0),
        })
    }

    fn capacity(&self) -> u64 {
        (self.cells.len() / 2) as u64
    }

    /// Frames queued and not yet read (a pending discard counts as read).
    pub fn queued(&self) -> u64 {
        let tail = self.tail.load(Ordering::Acquire).max(self.clear_upto.load(Ordering::Acquire));
        self.head.load(Ordering::Acquire).saturating_sub(tail)
    }

    /// Frames of room.
    pub fn room(&self) -> u64 {
        self.capacity() - self.queued().min(self.capacity())
    }

    /// Pushes as many whole frames of interleaved S16 stereo as fit; returns the frames taken.
    pub fn push(&self, samples: &[i16]) -> usize {
        let frames = (samples.len() / 2).min(self.room() as usize);
        let head = self.head.load(Ordering::Relaxed);
        let cap = self.capacity();
        for i in 0..frames {
            let at = ((head + i as u64) % cap) as usize * 2;
            self.cells[at].store((samples[i * 2] as f32 / 32768.0).to_bits(), Ordering::Relaxed);
            self.cells[at + 1].store((samples[i * 2 + 1] as f32 / 32768.0).to_bits(), Ordering::Relaxed);
        }
        self.head.store(head + frames as u64, Ordering::Release);
        frames
    }

    /// Reader side: the next frame, or None when empty. Honours a pending discard.
    pub fn pop(&self) -> Option<(f32, f32)> {
        let mut tail = self.tail.load(Ordering::Relaxed);
        let skip = self.clear_upto.load(Ordering::Acquire);
        if tail < skip {
            tail = skip;
        }
        if tail >= self.head.load(Ordering::Acquire) {
            self.tail.store(tail, Ordering::Release);
            return None;
        }
        let at = (tail % self.capacity()) as usize * 2;
        let l = f32::from_bits(self.cells[at].load(Ordering::Relaxed));
        let r = f32::from_bits(self.cells[at + 1].load(Ordering::Relaxed));
        self.tail.store(tail + 1, Ordering::Release);
        Some((l, r))
    }

    /// Writer side: drops everything queued and restarts the played count.
    pub fn discard(&self) {
        let head = self.head.load(Ordering::Acquire);
        self.base.store(head, Ordering::Release);
        self.clear_upto.store(head, Ordering::Release);
    }

    /// Frames read since the last discard.
    pub fn played(&self) -> u64 {
        self.tail.load(Ordering::Acquire).saturating_sub(self.base.load(Ordering::Acquire))
    }

    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Release);
    }

    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }
}

/// The IO procedure's state: the ring plus the linear resampler (source rate over device rate).
pub struct Renderer {
    ring: Arc<Ring>,
    step: f64,
    phase: f64,
    a: (f32, f32),
    b: (f32, f32),
    primed: bool,
}

impl Renderer {
    pub fn new(ring: Arc<Ring>, source_rate: f64, device_rate: f64) -> Renderer {
        let step = if device_rate > 0.0 { source_rate / device_rate } else { 1.0 };
        Renderer { ring, step, phase: 0.0, a: (0.0, 0.0), b: (0.0, 0.0), primed: false }
    }

    /// The next output frame, or None on an underrun (the caller writes silence).
    fn next_frame(&mut self) -> Option<(f32, f32)> {
        if (self.step - 1.0).abs() < 1e-9 {
            return self.ring.pop();
        }
        if !self.primed {
            self.a = self.ring.pop()?;
            self.b = self.ring.pop().unwrap_or(self.a);
            self.primed = true;
        }
        let p = self.phase as f32;
        let out = (self.a.0 + (self.b.0 - self.a.0) * p, self.a.1 + (self.b.1 - self.a.1) * p);
        self.phase += self.step;
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.a = self.b;
            match self.ring.pop() {
                Some(f) => self.b = f,
                None => {
                    // starved: prime again once data arrives
                    self.primed = false;
                    self.phase = 0.0;
                    return Some(out);
                }
            }
        }
        Some(out)
    }

    /// Fills the output list for one cycle: silence while paused or starved.
    pub fn render(&mut self, out: &mut BufferList) {
        let frames = out.frames() as usize;
        let paused = self.ring.paused.load(Ordering::Acquire);
        for f in 0..frames {
            let (l, r) = if paused {
                (0.0, 0.0)
            } else {
                match self.next_frame() {
                    Some(v) => v,
                    None => {
                        self.ring.underruns.fetch_add(1, Ordering::Relaxed);
                        (0.0, 0.0)
                    }
                }
            };
            let mut channel = 0u32;
            for buf in out.buffers.iter_mut() {
                let n = buf.channels.max(1) as usize;
                for c in 0..n {
                    let v = match channel + c as u32 {
                        0 => l,
                        1 => r,
                        _ => 0.0,
                    };
                    if let Some(slot) = buf.data.get_mut(f * n + c) {
                        *slot = v;
                    }
                }
                channel += n as u32;
            }
        }
    }
}

fn io_err(what: &str, e: AudioError) -> io::Error {
    io::Error::other(format!("{what}: {e:?}"))
}

/// An open stream on the sound server.
struct Stream {
    device: Device,
    proc_id: IoProcId,
    ring: Arc<Ring>,
}

impl Stream {
    fn open(role: Role, source_rate: u32) -> io::Result<Stream> {
        let system = System::connect(CLIENT).map_err(|e| io_err("connect to the audio server", e))?;
        let device = system.default_device(role).map_err(|e| io_err("default output device", e))?;
        let rate = match device.get(PropertyAddress::global(props::NOMINAL_RATE)) {
            Ok(PropertyValue::F64(r)) => r,
            _ => source_rate as f64,
        };
        let ring = Ring::new(RING_FRAMES);
        let mut renderer = Renderer::new(ring.clone(), source_rate as f64, rate);
        let proc_id = device
            .add_io_proc(
                IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: 0 },
                Box::new(move |_ctx: &IoContext, _now: &TimeStamp, _input: &BufferList, _input_ts: &TimeStamp, out: &mut BufferList, _out_ts: &TimeStamp| renderer.render(out)),
            )
            .map_err(|e| io_err("add the IO procedure", e))?;
        if let Err(e) = device.start(proc_id) {
            let _ = device.remove_io_proc(proc_id);
            return Err(io_err("start the device", e));
        }
        Ok(Stream { device, proc_id, ring })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.device.stop(self.proc_id);
        let _ = self.device.remove_io_proc(self.proc_id);
    }
}

struct Session {
    role: Role,
    rate: u32,
    desk: Option<Stream>,
    /// the last write, pause or discard
    active: Instant,
    meter: Meter,
    reported: Instant,
    paused: bool,
}

impl Session {
    /// Opens the stream if that has not happened yet.
    fn ensure(&mut self) -> io::Result<()> {
        if self.desk.is_some() {
            return Ok(());
        }
        let s = Stream::open(self.role, self.rate)?;
        s.ring.set_paused(self.paused);
        self.desk = Some(s);
        eprintln!("audio.speaker: opened the {:?} device of the audio server", self.role);
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.desk.is_some()
    }

    /// One trace line a second while playing: input level, throughput and the played count.
    fn report(&mut self) {
        let m = self.meter.take();
        self.reported = Instant::now();
        let (played, under) = self.desk.as_ref().map_or((0, 0), |d| (d.ring.played(), d.ring.underruns()));
        eprintln!("audio.speaker: trace {} frames in {} writes, peak {}, slowest write {:?}, played {played}, underruns {under}", m.frames, m.writes, m.peak, m.slowest);
    }

    fn release(&mut self) {
        self.desk = None;
    }
}

/// The speaker as an `AudioOut`.
pub struct SpeakerOut(Arc<Mutex<Session>>);

impl SpeakerOut {
    /// Music and video: the default output device. `channels` is the source's (always stereo today).
    pub fn new(rate: u32, channels: u32) -> Self {
        Self::with_role(Role::Output, rate, channels)
    }

    /// Lock and keyboard clicks and alerts: the sound-effects device role.
    pub fn system_sounds(rate: u32, channels: u32) -> Self {
        Self::with_role(Role::SoundEffects, rate, channels)
    }

    pub fn with_role(role: Role, rate: u32, _channels: u32) -> Self {
        let session = Arc::new(Mutex::new(Session { role, rate, desk: None, active: Instant::now(), meter: Meter::default(), reported: Instant::now(), paused: false }));
        let weak: Weak<Mutex<Session>> = Arc::downgrade(&session);
        std::thread::Builder::new()
            .name("speaker-idle".into())
            .spawn(move || {
                while let Some(session) = weak.upgrade() {
                    {
                        let mut s = session.lock().unwrap();
                        if s.is_open() && s.active.elapsed() >= IDLE_CLOSE {
                            s.release();
                            eprintln!("audio.speaker: idle, closed");
                        }
                    }
                    drop(session);
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .ok();
        SpeakerOut(session)
    }
}

/// Pushes `frames` into the ring, waiting for room; an error when nobody reads for `stall`.
fn push_all(ring: &Ring, frames: &[i16], stall: Duration) -> io::Result<()> {
    let mut rest = frames;
    let began = Instant::now();
    loop {
        let n = ring.push(rest);
        rest = &rest[n * 2..];
        if rest.len() < 2 {
            return Ok(());
        }
        if began.elapsed() > stall {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the audio server stopped taking frames"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl AudioOut for SpeakerOut {
    fn write(&mut self, frames: &[i16]) -> io::Result<()> {
        let began = Instant::now();
        let ring = {
            let mut s = self.0.lock().unwrap();
            s.ensure()?;
            s.desk.as_ref().expect("opened above").ring.clone()
        };
        let r = push_all(&ring, frames, WRITE_STALL);
        let mut s = self.0.lock().unwrap();
        s.meter.add(frames, began.elapsed());
        if let Err(e) = &r {
            eprintln!("audio.speaker: write failed after {:?}: {e}", began.elapsed());
            // start over on the next write
            s.release();
        }
        s.active = Instant::now();
        if s.reported.elapsed() >= REPORT_EVERY {
            s.report();
        }
        r
    }

    fn played(&mut self) -> u64 {
        self.0.lock().unwrap().desk.as_ref().map_or(0, |d| d.ring.played())
    }

    fn pause(&mut self, on: bool) {
        let mut s = self.0.lock().unwrap();
        s.paused = on;
        if let Some(d) = &s.desk {
            d.ring.set_paused(on);
        }
        s.active = Instant::now();
    }

    fn flush(&mut self) {
        // let what is queued play out, bounded, then drop the rest
        let ring = self.0.lock().unwrap().desk.as_ref().map(|d| d.ring.clone());
        if let Some(ring) = ring {
            let until = Instant::now() + Duration::from_millis(600);
            while ring.queued() > 0 && !self.0.lock().unwrap().paused && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            ring.discard();
        }
        self.0.lock().unwrap().active = Instant::now();
    }

    fn discard(&mut self) {
        let mut s = self.0.lock().unwrap();
        if let Some(d) = &s.desk {
            d.ring.discard();
        }
        s.active = Instant::now();
    }
}

impl Drop for SpeakerOut {
    fn drop(&mut self) {
        if let Ok(mut s) = self.0.lock() {
            if s.is_open() {
                s.release();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;

    #[test]
    fn meter_tracks_frames_peak_and_slowest_write() {
        let mut m = Meter::default();
        m.add(&[0, 0, -300, 120], Duration::from_millis(3));
        m.add(&[10, 10], Duration::from_millis(1));
        assert_eq!(m.frames, 3);
        assert_eq!(m.peak, 300);
        assert_eq!(m.writes, 2);
        assert_eq!(m.slowest, Duration::from_millis(3));
    }

    #[test]
    fn meter_take_resets() {
        let mut m = Meter::default();
        m.add(&[i16::MIN, 0], Duration::ZERO);
        assert_eq!(m.take().peak, 32768);
        assert_eq!(m, Meter::default());
    }

    #[test]
    fn silence_has_zero_peak() {
        let mut m = Meter::default();
        m.add(&[0; 8], Duration::ZERO);
        assert_eq!(m.peak, 0);
    }

    fn stereo(n: usize, v: i16) -> Vec<i16> {
        vec![v; n * 2]
    }

    fn list(frames: usize) -> BufferList {
        BufferList { buffers: vec![Buffer { channels: 1, data: vec![9.0; frames] }, Buffer { channels: 1, data: vec![9.0; frames] }] }
    }

    #[test]
    fn ring_round_trips_in_order_and_scales_to_float() {
        let r = Ring::new(4);
        assert_eq!(r.push(&[16384, -16384, 8192, 0]), 2);
        assert_eq!(r.pop(), Some((0.5, -0.5)));
        assert_eq!(r.pop(), Some((0.25, 0.0)));
        assert_eq!(r.pop(), None);
        assert_eq!(r.played(), 2);
    }

    #[test]
    fn ring_takes_only_what_fits_and_wraps() {
        let r = Ring::new(4);
        assert_eq!(r.push(&stereo(6, 100)), 4);
        assert_eq!(r.room(), 0);
        assert!(r.pop().is_some());
        assert_eq!(r.push(&stereo(3, -100)), 1);
        let mut got = 0;
        while r.pop().is_some() {
            got += 1;
        }
        assert_eq!(got, 4);
    }

    #[test]
    fn discard_drops_queued_frames_and_restarts_the_count() {
        let r = Ring::new(8);
        r.push(&stereo(5, 1000));
        r.pop();
        r.pop();
        assert_eq!(r.played(), 2);
        r.discard();
        assert_eq!(r.queued(), 0);
        assert_eq!(r.pop(), None);
        assert_eq!(r.played(), 0);
        r.push(&stereo(1, 3000));
        assert!(r.pop().is_some());
        assert_eq!(r.played(), 1);
    }

    #[test]
    fn renderer_copies_at_equal_rates_and_pads_with_silence() {
        let ring = Ring::new(8);
        ring.push(&[16384, -16384, 8192, 4096]);
        let mut rd = Renderer::new(ring.clone(), 48_000.0, 48_000.0);
        let mut out = list(3);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.25, 0.0]);
        assert_eq!(out.buffers[1].data, vec![-0.5, 0.125, 0.0]);
        assert_eq!(ring.underruns(), 1);
    }

    #[test]
    fn renderer_is_silent_while_paused_and_keeps_the_queue() {
        let ring = Ring::new(8);
        ring.push(&stereo(2, 16384));
        ring.set_paused(true);
        let mut rd = Renderer::new(ring.clone(), 48_000.0, 48_000.0);
        let mut out = list(2);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.0, 0.0]);
        assert_eq!(ring.queued(), 2);
    }

    #[test]
    fn renderer_upsamples_linearly_when_the_device_runs_twice_as_fast() {
        let ring = Ring::new(8);
        // left: 0, 0.25, 0.5; the midpoints are 0.125 and 0.375
        ring.push(&[0, 0, 8192, 8192, 16384, 16384]);
        let mut rd = Renderer::new(ring, 24_000.0, 48_000.0);
        let mut out = list(4);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.0, 0.125, 0.25, 0.375]);
    }

    #[test]
    fn extra_device_channels_get_silence_and_interleaved_buffers_fill() {
        let ring = Ring::new(4);
        ring.push(&[16384, 8192]);
        let mut rd = Renderer::new(ring, 48_000.0, 48_000.0);
        let mut out = BufferList { buffers: vec![Buffer { channels: 4, data: vec![9.0; 4] }] };
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn push_all_times_out_when_nobody_reads() {
        let ring = Ring::new(2);
        let e = push_all(&ring, &stereo(5, 1), Duration::from_millis(20)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert_eq!(ring.queued(), 2);
    }
}
