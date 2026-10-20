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

/// An error that names the device it happened on (its object id and uid), so a log line says which output failed.
fn device_err(what: &str, device: &Device, e: AudioError) -> io::Error {
    let uid = match device.get(PropertyAddress::global(props::DEVICE_UID)) {
        Ok(PropertyValue::String(u)) => u,
        _ => "unknown uid".to_string(),
    };
    io_err(&format!("{what} on device {} ({uid})", device.id.0), e)
}

/// An open stream on the sound server.
struct Stream {
    device: Device,
    proc_id: IoProcId,
    ring: Arc<Ring>,
}

impl Stream {
    /// Runs on an opener thread, never under the session lock: the Hello gets 100 ms, so a server that accepts and
    /// never answers costs the caller nothing.
    fn open(socket: &str, role: Role, source_rate: u32) -> io::Result<Stream> {
        let system = System::connect_path_within(socket, CLIENT, Duration::from_millis(100)).map_err(|e| io_err("connect to the audio server", e))?;
        Self::open_on(&system, role, source_rate)
    }

    /// The stream on the default device of `role` on a connected server: the device's rate, an IO procedure with the
    /// device's own buffer size, started. Every failure says which device it happened on.
    fn open_on(system: &System, role: Role, source_rate: u32) -> io::Result<Stream> {
        let device = system.default_device(role).map_err(|e| io_err(&format!("default {role:?} device"), e))?;
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
            .map_err(|e| device_err("add the IO procedure", &device, e))?;
        if let Err(e) = device.start(proc_id) {
            let _ = device.remove_io_proc(proc_id);
            return Err(device_err("start the device", &device, e));
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
    /// The control socket of the audio server.
    socket: String,
    role: Role,
    rate: u32,
    desk: Option<Stream>,
    /// the last write, pause or discard
    active: Instant,
    meter: Meter,
    reported: Instant,
    paused: bool,
    /// No new connection before this; set after a failed open (1 s, doubling to 30 s), cleared by a good one.
    retry_after: Option<Instant>,
    backoff: Duration,
    /// Why the last open failed, for the error a caller gets while the retry is pending.
    last_error: Option<String>,
    /// An open in flight on its own thread; polled by `ensure`, so no caller waits for the server.
    opening: Option<std::sync::mpsc::Receiver<io::Result<Stream>>>,
}

impl Session {
    /// Opens the stream if that has not happened yet. Never waits for the audio server: the open runs on its own
    /// thread and this returns `WouldBlock` until it has finished (the caller drops the frames and writes again).
    fn ensure(&mut self) -> io::Result<()> {
        if self.desk.is_some() {
            return Ok(());
        }
        if let Some(rx) = &self.opening {
            match rx.try_recv() {
                Ok(Ok(s)) => {
                    self.opening = None;
                    self.retry_after = None;
                    self.last_error = None;
                    self.backoff = Duration::from_secs(1);
                    s.ring.set_paused(self.paused);
                    self.desk = Some(s);
                    eprintln!("audio.speaker: opened the {:?} device of the audio server", self.role);
                    return Ok(());
                }
                Ok(Err(e)) => {
                    self.opening = None;
                    eprintln!("audio.speaker: could not open the {:?} device of the audio server: {e}", self.role);
                    self.last_error = Some(e.to_string());
                    self.retry_after = Some(Instant::now() + self.backoff);
                    self.backoff = (self.backoff * 2).min(Duration::from_secs(30));
                    return Err(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return Err(io::Error::new(io::ErrorKind::WouldBlock, "opening the audio server stream")),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.opening = None;
                    self.retry_after = Some(Instant::now() + self.backoff);
                    return Err(io::Error::other("the stream opener ended"));
                }
            }
        }
        if let Some(at) = self.retry_after {
            if Instant::now() < at {
                // the open that failed said why (no server, no default device, a refused IO procedure); say that
                let why = self.last_error.as_deref().unwrap_or("the audio server is not reachable");
                return Err(io::Error::new(io::ErrorKind::NotConnected, format!("{why}; retrying later")));
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (role, rate, socket) = (self.role, self.rate, self.socket.clone());
        std::thread::Builder::new().name("speaker-open".into()).spawn(move || {
            let _ = tx.send(Stream::open(&socket, role, rate));
        })?;
        self.opening = Some(rx);
        Err(io::Error::new(io::ErrorKind::WouldBlock, "opening the audio server stream"))
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

    /// Closes the stream. Dropping it asks the server to stop and remove the IO proc (up to 5 s each when the
    /// server is wedged), so the drop happens on its own thread.
    fn release(&mut self) {
        if let Some(d) = self.desk.take() {
            if std::thread::Builder::new().name("speaker-close".into()).spawn(move || drop(d)).is_err() {
                eprintln!("audio.speaker: could not spawn the closer thread");
            }
        }
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

    pub fn with_role(role: Role, rate: u32, channels: u32) -> Self {
        Self::with_socket(bifrost_audio_types::control::CONTROL_SOCKET, role, rate, channels)
    }

    /// Like `with_role` on another server socket (a private server in tests).
    pub fn with_socket(socket: &str, role: Role, rate: u32, _channels: u32) -> Self {
        let session = Arc::new(Mutex::new(Session { socket: socket.to_string(), role, rate, desk: None, active: Instant::now(), meter: Meter::default(), reported: Instant::now(), paused: false, retry_after: None, backoff: Duration::from_secs(1), last_error: None, opening: None }));
        let weak: Weak<Mutex<Session>> = Arc::downgrade(&session);
        std::thread::Builder::new()
            .name("speaker-idle".into())
            .spawn(move || {
                while let Some(session) = weak.upgrade() {
                    {
                        let mut s = session.lock().unwrap_or_else(|p| p.into_inner());
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
            let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
            s.ensure()?;
            s.desk.as_ref().expect("opened above").ring.clone()
        };
        let r = push_all(&ring, frames, WRITE_STALL);
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
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
        self.0.lock().unwrap_or_else(|p| p.into_inner()).desk.as_ref().map_or(0, |d| d.ring.played())
    }

    fn pause(&mut self, on: bool) {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        s.paused = on;
        if let Some(d) = &s.desk {
            d.ring.set_paused(on);
        }
        s.active = Instant::now();
    }

    fn flush(&mut self) {
        // let what is queued play out, bounded, then drop the rest
        let ring = self.0.lock().unwrap_or_else(|p| p.into_inner()).desk.as_ref().map(|d| d.ring.clone());
        if let Some(ring) = ring {
            let until = Instant::now() + Duration::from_millis(600);
            while ring.queued() > 0 && !self.0.lock().unwrap_or_else(|p| p.into_inner()).paused && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            ring.discard();
        }
        self.0.lock().unwrap_or_else(|p| p.into_inner()).active = Instant::now();
    }

    fn discard(&mut self) {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
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

    /// A real control loop (the server's `serve`) on a private socket, with one null device, for the output-path tests.
    struct TestServer {
        path: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestServer {
        /// `transport` "pci" makes the device a real playback device for the default choice; "virtual" never is.
        fn start(tag: &str, transport: &str) -> TestServer {
            use bifrost_audio_driver::null::NullDriver;
            use bifrost_audiod::control::{listen, serve, CoreConfig, NoObjects, ServerCore};
            let dir = std::env::temp_dir().join(format!("bifrost-speaker-test-{}-{tag}", std::process::id()));
            let path = dir.join("control").to_string_lossy().into_owned();
            let listener = listen(&path).expect("listen on the private socket");
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            let transport = transport.to_string();
            let thread = std::thread::spawn(move || {
                let mut desc = NullDriver::stereo("alsa:PCH:0", 48_000.0);
                desc.name = "ALC233 Analog".into();
                desc.transport = transport;
                let mut core = ServerCore::new(vec![Box::new(NullDriver::new(vec![desc]))], Box::new(NoObjects), CoreConfig::default());
                serve(&mut core, &listener, &stop2, None);
            });
            TestServer { path, stop, thread: Some(thread) }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Writes until the stream has opened (the open runs on its own thread, so the first writes are `WouldBlock`).
    fn write_when_open(out: &mut SpeakerOut, frames: &[i16], limit: Duration) -> io::Result<()> {
        let until = Instant::now() + limit;
        loop {
            match out.write(frames) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
                other => return other,
            }
        }
    }

    #[test]
    fn the_speaker_reaches_io_on_the_default_device_of_a_real_server() {
        let server = TestServer::start("io", "pci");
        // the media player's output (Role::Output) and the shell's system sounds (Role::SoundEffects) take the same path
        for role in [Role::Output, Role::SoundEffects] {
            let mut out = SpeakerOut::with_socket(&server.path, role, 48_000, 2);
            let frames = stereo(480, 2000);
            write_when_open(&mut out, &frames, Duration::from_secs(10)).unwrap_or_else(|e| panic!("{role:?}: the stream did not open: {e}"));
            // the server's cycles pull the frames out of the ring
            let until = Instant::now() + Duration::from_secs(5);
            while out.played() == 0 && Instant::now() < until {
                let _ = out.write(&frames);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(out.played() > 0, "{role:?}: the server never ran the IO procedure");
            drop(out);
            // the closer thread removes the IO procedure; the device is free for the next role
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    #[test]
    fn a_refused_open_says_which_step_and_why_not_that_the_server_is_unreachable() {
        // the only device is virtual, so the server publishes no default output
        let server = TestServer::start("nodefault", "virtual");
        let mut out = SpeakerOut::with_socket(&server.path, Role::Output, 48_000, 2);
        let e = write_when_open(&mut out, &stereo(64, 1), Duration::from_secs(10)).unwrap_err();
        assert!(e.to_string().contains("default Output device"), "{e}");
        // while the retry is pending the caller is told the same reason
        let again = out.write(&stereo(64, 1)).unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::NotConnected);
        assert!(again.to_string().contains("default Output device") && again.to_string().contains("retrying later"), "{again}");
    }
}
