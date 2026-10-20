//! Speaker output on the desktop: an IO procedure on the default output device of `bifrost-audiod`
//! (the HAL client library), fed from a lock-free ring. No libasound, no `/dev/snd` here. The
//! stream opens on the first write and is released again after `IDLE_CLOSE` without playback so
//! a paused or finished player does not keep the device running (opening the Music app alone
//! never does). System sounds use the sound-effects role (`SpeakerOut::system_sounds`).
//!
//! The IO procedure never allocates, locks or makes a syscall: it reads the ring (atomics only),
//! resamples linearly when the device rate differs from the source rate, and writes the
//! buffers of the cycle. No lock is shared between it and the player thread: the session mutex is
//! taken by the writer, the idle thread and `played`, never by the IO procedure.
//!
//! Start-up and recovery are prebuffered. The IO procedure is added when the stream opens but the
//! device is started only once the ring holds `prefill` frames (about 50 ms, or the first write of
//! a short system sound), so the first cycles find data. After a discard (a seek, a skip, a
//! reconnect) and after a starve the IO procedure plays silence until the ring has refilled to the
//! prefill, instead of stuttering through the refill. What it counts is split three ways
//! (`RingStats`): underrun events (the ring ran dry while playing), the silent frames of those
//! underruns, and the silent frames spent prebuffering (not glitches).

use super::audio_out::AudioOut;
use bifrost_audio_hal::{Device, IoContext, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::{props, AudioError, BufferList, PropertyAddress, PropertyValue, TimeStamp};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The client name the media player says hello with.
pub const CLIENT: &str = "is.olibuijr.mediaplayer";

/// Playback idle for this long closes the stream.
const IDLE_CLOSE: Duration = Duration::from_secs(5);

const REPORT_EVERY: Duration = Duration::from_secs(1);

/// The device starts at the latest this long after the first write, whatever the ring holds (a short sound).
const START_MAX_WAIT: Duration = Duration::from_millis(60);

/// How the stream's device start stands: the IO procedure is added at open and started on the prefill.
const START_PENDING: u8 = 0;
const START_STARTING: u8 = 1;
const START_RUNNING: u8 = 2;
const START_FAILED: u8 = 3;

/// Silent frames past this many in one hold are not counted: the writer has stopped, nothing is glitching.
const HOLD_COUNT_CAP: u64 = 48_000;

/// Source frames the ring holds before playback starts or resumes: 50 ms for music and video, a single frame for
/// system sounds (they are written whole, and a click must not wait).
fn prefill_frames(role: Role, rate: u32) -> u64 {
    if matches!(role, Role::Output) {
        (rate as u64 / 20).max(1)
    } else {
        1
    }
}

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
    /// writes that had to sleep for room (a full ring: expected back-pressure) and the time they slept
    waited_writes: u32,
    waited: Duration,
}

impl Meter {
    /// `took` is the whole write, `waited` the part of it spent asleep waiting for room in the ring.
    fn add(&mut self, samples: &[i16], took: Duration, waited: Duration) {
        self.frames += samples.len() as u64 / 2;
        self.peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0).max(self.peak);
        self.writes += 1;
        self.slowest = self.slowest.max(took);
        if !waited.is_zero() {
            self.waited_writes += 1;
            self.waited += waited;
        }
    }

    fn take(&mut self) -> Meter {
        std::mem::take(self)
    }
}

/// What the IO procedure counted since the stream opened. Cumulative: a reporter takes `since` its last reading.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingStats {
    /// times the ring ran dry while playing (a glitch, or the end of what was written)
    pub underrun_events: u64,
    /// silent frames those underruns cost, until the ring refilled (capped at 1 s per underrun)
    pub underrun_frames: u64,
    /// silent frames while prebuffering at the start or after a discard: waiting, not a glitch
    pub prebuffer_frames: u64,
}

impl RingStats {
    /// The change since `earlier` (a reading of the same ring).
    pub fn since(&self, earlier: &RingStats) -> RingStats {
        RingStats {
            underrun_events: self.underrun_events.saturating_sub(earlier.underrun_events),
            underrun_frames: self.underrun_frames.saturating_sub(earlier.underrun_frames),
            prebuffer_frames: self.prebuffer_frames.saturating_sub(earlier.prebuffer_frames),
        }
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
    /// bumped by every discard: the reader notices and prebuffers again
    generation: AtomicU32,
    /// the writer is letting the queue play out (`flush`): play a short tail, and an empty ring is no underrun
    draining: AtomicBool,
    underrun_events: AtomicU64,
    underrun_frames: AtomicU64,
    prebuffer_frames: AtomicU64,
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
            generation: AtomicU32::new(0),
            draining: AtomicBool::new(false),
            underrun_events: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            prebuffer_frames: AtomicU64::new(0),
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
        self.draining.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Writer side: the queue is being played out before a discard (see `AudioOut::flush`).
    pub fn set_draining(&self, on: bool) {
        self.draining.store(on, Ordering::Release);
    }

    /// Frames read since the last discard.
    pub fn played(&self) -> u64 {
        self.tail.load(Ordering::Acquire).saturating_sub(self.base.load(Ordering::Acquire))
    }

    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Release);
    }

    /// Underrun events since the stream opened (see `RingStats`).
    pub fn underruns(&self) -> u32 {
        self.underrun_events.load(Ordering::Relaxed) as u32
    }

    pub fn stats(&self) -> RingStats {
        RingStats {
            underrun_events: self.underrun_events.load(Ordering::Relaxed),
            underrun_frames: self.underrun_frames.load(Ordering::Relaxed),
            prebuffer_frames: self.prebuffer_frames.load(Ordering::Relaxed),
        }
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
    /// source frames the ring must hold before playback starts or resumes (at least 1)
    prefill: u64,
    /// silent until the ring holds `prefill` frames (the start, a discard, a starve)
    hold: bool,
    /// frames the current hold has lasted
    held_run: u64,
    /// the hold follows an underrun (its silence is an underrun's cost), not a start or a discard
    after_starve: bool,
    /// the `Ring::generation` last seen
    gen_seen: u32,
}

impl Renderer {
    pub fn new(ring: Arc<Ring>, source_rate: f64, device_rate: f64) -> Renderer {
        let step = if device_rate > 0.0 { source_rate / device_rate } else { 1.0 };
        let gen_seen = ring.generation.load(Ordering::Acquire);
        Renderer { ring, step, phase: 0.0, a: (0.0, 0.0), b: (0.0, 0.0), primed: false, prefill: 1, hold: true, held_run: 0, after_starve: false, gen_seen }
    }

    /// Plays only once the ring holds `frames` source frames, at the start and again after a discard or a starve.
    pub fn with_prefill(mut self, frames: u64) -> Renderer {
        self.prefill = frames.max(1);
        self
    }

    /// Ends the hold when the ring has refilled. A short tail is not held for ever: with something queued the hold
    /// ends after twice the prefill (the writer has stopped, e.g. at the end of a song), or at once while draining.
    fn try_release(&mut self) -> bool {
        let queued = self.ring.queued();
        let ready = queued >= self.prefill
            || (queued > 0 && (self.held_run >= self.prefill.saturating_mul(2) || self.ring.draining.load(Ordering::Acquire)));
        if ready {
            self.hold = false;
            self.held_run = 0;
        }
        ready
    }

    fn count_held(&mut self) {
        self.held_run = self.held_run.saturating_add(1);
        if self.held_run <= HOLD_COUNT_CAP {
            let counter = if self.after_starve { &self.ring.underrun_frames } else { &self.ring.prebuffer_frames };
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The ring ran dry while playing: one event (none while draining), then silence until it has refilled.
    fn starve(&mut self) {
        self.hold = true;
        self.held_run = 0;
        self.primed = false;
        self.phase = 0.0;
        if self.ring.draining.load(Ordering::Acquire) {
            self.after_starve = false;
            self.ring.prebuffer_frames.fetch_add(1, Ordering::Relaxed);
        } else {
            self.after_starve = true;
            self.ring.underrun_events.fetch_add(1, Ordering::Relaxed);
            self.ring.underrun_frames.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// One output frame: silence while holding, else the next frame (a starve ends the cycle's data).
    fn frame(&mut self) -> (f32, f32) {
        if self.hold && !self.try_release() {
            self.count_held();
            return (0.0, 0.0);
        }
        match self.next_frame() {
            Some(v) => v,
            None => {
                self.starve();
                (0.0, 0.0)
            }
        }
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
        let generation = self.ring.generation.load(Ordering::Acquire);
        if generation != self.gen_seen {
            // a discard: what is queued now is new, so prebuffer it again
            self.gen_seen = generation;
            self.hold = true;
            self.held_run = 0;
            self.after_starve = false;
            self.primed = false;
            self.phase = 0.0;
        }
        for f in 0..frames {
            let (l, r) = if paused { (0.0, 0.0) } else { self.frame() };
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
    /// `START_*`: the device is started once the ring holds `prefill` frames (or `START_MAX_WAIT` after the first write)
    start: Arc<AtomicU8>,
    /// source frames the ring must hold before the device starts (and the IO procedure resumes after a starve)
    prefill: u64,
    first_write: Option<Instant>,
}

impl Stream {
    /// Runs on an opener thread, never under the session lock: the Hello gets 100 ms, so a server that accepts and
    /// never answers costs the caller nothing.
    fn open(socket: &str, role: Role, source_rate: u32) -> io::Result<Stream> {
        let system = System::connect_path_within(socket, CLIENT, Duration::from_millis(100)).map_err(|e| io_err("connect to the audio server", e))?;
        Self::open_on(&system, role, source_rate)
    }

    /// The stream on the default device of `role` on a connected server: the device's rate, an IO procedure with the
    /// device's own buffer size. The device is not started here but by the writer once the ring is prefilled
    /// (`Session::maybe_start`), so the first cycles never find an empty ring. Every failure says which device it
    /// happened on.
    fn open_on(system: &System, role: Role, source_rate: u32) -> io::Result<Stream> {
        let device = system.default_device(role).map_err(|e| io_err(&format!("default {role:?} device"), e))?;
        let rate = match device.get(PropertyAddress::global(props::NOMINAL_RATE)) {
            Ok(PropertyValue::F64(r)) => r,
            _ => source_rate as f64,
        };
        let ring = Ring::new(RING_FRAMES);
        let prefill = prefill_frames(role, source_rate);
        let mut renderer = Renderer::new(ring.clone(), source_rate as f64, rate).with_prefill(prefill);
        let proc_id = device
            .add_io_proc(
                IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: 0 },
                Box::new(move |_ctx: &IoContext, _now: &TimeStamp, _input: &BufferList, _input_ts: &TimeStamp, out: &mut BufferList, _out_ts: &TimeStamp| renderer.render(out)),
            )
            .map_err(|e| device_err("add the IO procedure", &device, e))?;
        Ok(Stream { device, proc_id, ring, start: Arc::new(AtomicU8::new(START_PENDING)), prefill, first_write: None })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.start.load(Ordering::Acquire) != START_PENDING {
            let _ = self.device.stop(self.proc_id);
        }
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
    /// the ring's counters at the last report, to print the interval's change
    last_stats: RingStats,
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
        if let Some(d) = &self.desk {
            if d.start.load(Ordering::Acquire) != START_FAILED {
                return Ok(());
            }
            // the device did not start (the thread that tried has said why): start over after the backoff
            self.release();
            self.last_error = Some("the device did not start".to_string());
            self.retry_after = Some(Instant::now() + self.backoff);
            self.backoff = (self.backoff * 2).min(Duration::from_secs(30));
        }
        if let Some(rx) = &self.opening {
            match rx.try_recv() {
                Ok(Ok(s)) => {
                    self.opening = None;
                    self.retry_after = None;
                    self.last_error = None;
                    self.backoff = Duration::from_secs(1);
                    s.ring.set_paused(self.paused);
                    self.last_stats = RingStats::default();
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

    /// Starts the device of an open stream once it is due: the ring holds the prefill, the first write is
    /// `START_MAX_WAIT` old, or `force` (a writer that waits for room, a flush). Never with an empty ring. The start
    /// is a round trip to the server, so it runs on its own thread; the stream's `start` word says how it went.
    fn maybe_start(&mut self, force: bool) {
        let Some(d) = self.desk.as_mut() else { return };
        if d.start.load(Ordering::Acquire) != START_PENDING {
            return;
        }
        let queued = d.ring.queued();
        let waited = d.first_write.is_some_and(|t| t.elapsed() >= START_MAX_WAIT);
        if queued == 0 || !(force || waited || queued >= d.prefill) {
            return;
        }
        d.start.store(START_STARTING, Ordering::Release);
        let (device, id, state) = (d.device.clone(), d.proc_id, d.start.clone());
        let spawned = std::thread::Builder::new().name("speaker-start".into()).spawn(move || match device.start(id) {
            Ok(()) => state.store(START_RUNNING, Ordering::Release),
            Err(e) => {
                eprintln!("audio.speaker: {}", device_err("start the device", &device, e));
                state.store(START_FAILED, Ordering::Release);
            }
        });
        if spawned.is_err() {
            eprintln!("audio.speaker: could not spawn the starter thread");
            d.start.store(START_FAILED, Ordering::Release);
        }
    }

    /// After a successful write: remembers when the first one was and starts the device if it is due. True when this
    /// was the first write and the device still waits for more: the caller arms the `START_MAX_WAIT` deadline.
    fn note_write(&mut self) -> bool {
        let mut first = false;
        if let Some(d) = self.desk.as_mut() {
            if d.first_write.is_none() {
                d.first_write = Some(Instant::now());
                first = true;
            }
        }
        self.maybe_start(false);
        first && self.desk.as_ref().is_some_and(|d| d.start.load(Ordering::Acquire) == START_PENDING)
    }

    /// One trace line a second while playing: input level, throughput, the played count and what the IO procedure
    /// counted this interval (`underruns` are events and their silent frames; `prebuffer` is waiting, not a glitch),
    /// then the totals since the stream opened. `slowest write` includes the time asleep for room in a full ring,
    /// which is back-pressure and expected (`asleep`).
    fn report(&mut self) {
        let m = self.meter.take();
        self.reported = Instant::now();
        let (played, fill, stats) = self.desk.as_ref().map_or((0, 0, RingStats::default()), |d| (d.ring.played(), d.ring.queued(), d.ring.stats()));
        let d = stats.since(&self.last_stats);
        self.last_stats = stats;
        eprintln!(
            "audio.speaker: trace {} frames in {} writes ({} waited for room, {:?} asleep), peak {}, slowest write {:?}, fill {fill}, played {played}, underruns {} event(s) / {} frame(s), prebuffer {} frame(s) this interval, total {} event(s) / {} frame(s)",
            m.frames, m.writes, m.waited_writes, m.waited, m.peak, m.slowest, d.underrun_events, d.underrun_frames, d.prebuffer_frames, stats.underrun_events, stats.underrun_frames
        );
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
        let session = Arc::new(Mutex::new(Session { socket: socket.to_string(), role, rate, desk: None, active: Instant::now(), meter: Meter::default(), reported: Instant::now(), last_stats: RingStats::default(), paused: false, retry_after: None, backoff: Duration::from_secs(1), last_error: None, opening: None }));
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

/// Pushes `frames` into the ring, waiting for room; an error when nobody reads for `stall`. `on_wait` runs before each
/// sleep (the caller starts a device that is still waiting for its prefill: a write larger than the ring would
/// otherwise wait for a reader that never starts). Returns the time spent asleep waiting for room.
fn push_all(ring: &Ring, frames: &[i16], stall: Duration, on_wait: &mut dyn FnMut()) -> io::Result<Duration> {
    let mut rest = frames;
    let began = Instant::now();
    let mut waited = Duration::ZERO;
    loop {
        let n = ring.push(rest);
        rest = &rest[n * 2..];
        if rest.len() < 2 {
            return Ok(waited);
        }
        if began.elapsed() > stall {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the audio server stopped taking frames"));
        }
        on_wait();
        let slept = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        waited += slept.elapsed();
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
        let session = self.0.clone();
        let mut start_if_due = || session.lock().unwrap_or_else(|p| p.into_inner()).maybe_start(true);
        let r = push_all(&ring, frames, WRITE_STALL, &mut start_if_due);
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        s.meter.add(frames, began.elapsed(), r.as_ref().map_or(Duration::ZERO, |w| *w));
        if let Err(e) = &r {
            eprintln!("audio.speaker: write failed after {:?}: {e}", began.elapsed());
            // start over on the next write
            s.release();
        } else if s.note_write() {
            // a first write that left the device waiting for its prefill: start it by the deadline whatever follows
            let weak = Arc::downgrade(&self.0);
            let armed = std::thread::Builder::new().name("speaker-deadline".into()).spawn(move || {
                std::thread::sleep(START_MAX_WAIT + Duration::from_millis(5));
                if let Some(session) = weak.upgrade() {
                    session.lock().unwrap_or_else(|p| p.into_inner()).maybe_start(false);
                }
            });
            if armed.is_err() {
                s.maybe_start(true);
            }
        }
        s.active = Instant::now();
        if s.reported.elapsed() >= REPORT_EVERY {
            s.report();
        }
        r.map(|_| ())
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
        // let what is queued play out, bounded, then drop the rest; a stream still waiting for its prefill starts now
        let ring = {
            let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
            s.maybe_start(true);
            s.desk.as_ref().map(|d| d.ring.clone())
        };
        if let Some(ring) = ring {
            // a short tail plays instead of waiting for a prefill, and the empty ring at the end is no underrun
            ring.set_draining(true);
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
        m.add(&[0, 0, -300, 120], Duration::from_millis(3), Duration::from_millis(2));
        m.add(&[10, 10], Duration::from_millis(1), Duration::ZERO);
        assert_eq!(m.frames, 3);
        assert_eq!(m.peak, 300);
        assert_eq!(m.writes, 2);
        assert_eq!(m.slowest, Duration::from_millis(3));
        // only the write that slept for room counts as waiting
        assert_eq!((m.waited_writes, m.waited), (1, Duration::from_millis(2)));
    }

    #[test]
    fn meter_take_resets() {
        let mut m = Meter::default();
        m.add(&[i16::MIN, 0], Duration::ZERO, Duration::ZERO);
        assert_eq!(m.take().peak, 32768);
        assert_eq!(m, Meter::default());
    }

    #[test]
    fn silence_has_zero_peak() {
        let mut m = Meter::default();
        m.add(&[0; 8], Duration::ZERO, Duration::ZERO);
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
        assert_eq!(ring.stats(), RingStats { underrun_events: 1, underrun_frames: 1, prebuffer_frames: 0 });
    }

    fn held(prefill: u64) -> (Arc<Ring>, Renderer) {
        let ring = Ring::new(64);
        let rd = Renderer::new(ring.clone(), 48_000.0, 48_000.0).with_prefill(prefill);
        (ring, rd)
    }

    #[test]
    fn the_renderer_waits_for_the_prefill_and_counts_that_apart_from_underruns() {
        let (ring, mut rd) = held(4);
        ring.push(&stereo(2, 16384));
        let mut out = list(2);
        rd.render(&mut out);
        // 2 of 4 frames queued: silence, the queue is kept, and the wait is prebuffering, not an underrun
        assert_eq!(out.buffers[0].data, vec![0.0, 0.0]);
        assert_eq!(ring.queued(), 2);
        assert_eq!(ring.stats(), RingStats { underrun_events: 0, underrun_frames: 0, prebuffer_frames: 2 });
        ring.push(&stereo(3, 16384));
        let mut out = list(3);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.5, 0.5]);
        assert_eq!(ring.stats(), RingStats { underrun_events: 0, underrun_frames: 0, prebuffer_frames: 2 });
    }

    #[test]
    fn a_starve_is_one_event_then_silence_until_the_ring_has_refilled() {
        let (ring, mut rd) = held(4);
        ring.push(&stereo(4, 16384));
        let mut out = list(6);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        // frame 4 found the ring dry (the event and its first silent frame), frame 5 was held
        assert_eq!(ring.stats(), RingStats { underrun_events: 1, underrun_frames: 2, prebuffer_frames: 0 });
        // still empty: more silence, the same single event
        let mut out = list(2);
        rd.render(&mut out);
        assert_eq!(ring.stats(), RingStats { underrun_events: 1, underrun_frames: 4, prebuffer_frames: 0 });
        // 2 frames are below the prefill of 4: held; 4 play
        ring.push(&stereo(2, 16384));
        let mut out = list(1);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.0]);
        ring.push(&stereo(2, 16384));
        let mut out = list(2);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.5]);
        assert_eq!(ring.stats().underrun_events, 1);
    }

    #[test]
    fn a_discard_prebuffers_again() {
        let (ring, mut rd) = held(2);
        ring.push(&stereo(4, 16384));
        let mut out = list(1);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5]);
        ring.discard();
        ring.push(&stereo(1, 16384));
        let mut out = list(2);
        rd.render(&mut out);
        // one frame of the new material is below the prefill of 2
        assert_eq!(out.buffers[0].data, vec![0.0, 0.0]);
        assert_eq!(ring.stats(), RingStats { underrun_events: 0, underrun_frames: 0, prebuffer_frames: 2 });
        ring.push(&stereo(1, 16384));
        let mut out = list(2);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.5]);
    }

    #[test]
    fn draining_plays_a_short_tail_and_the_empty_ring_after_it_is_no_underrun() {
        let (ring, mut rd) = held(8);
        ring.push(&stereo(3, 16384));
        ring.set_draining(true);
        let mut out = list(5);
        rd.render(&mut out);
        assert_eq!(out.buffers[0].data, vec![0.5, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!((ring.stats().underrun_events, ring.stats().underrun_frames), (0, 0));
        // a discard ends the drain
        ring.discard();
        assert!(!ring.draining.load(Ordering::Acquire));
    }

    #[test]
    fn a_short_tail_plays_after_a_bounded_hold_when_the_writer_has_stopped() {
        let (ring, mut rd) = held(4);
        ring.push(&stereo(1, 16384));
        let mut out = list(9);
        rd.render(&mut out);
        // held for twice the prefill (8 frames), then the tail plays
        assert_eq!(out.buffers[0].data[..8], [0.0; 8]);
        assert_eq!(out.buffers[0].data[8], 0.5);
    }

    #[test]
    fn stats_report_the_change_since_the_last_reading() {
        let a = RingStats { underrun_events: 1, underrun_frames: 10, prebuffer_frames: 100 };
        let b = RingStats { underrun_events: 3, underrun_frames: 10, prebuffer_frames: 150 };
        assert_eq!(b.since(&a), RingStats { underrun_events: 2, underrun_frames: 0, prebuffer_frames: 50 });
        assert_eq!(b.since(&b), RingStats::default());
        // a reading of a newer ring (counters back at 0) never underflows
        assert_eq!(RingStats::default().since(&b), RingStats::default());
    }

    #[test]
    fn the_prefill_is_fifty_milliseconds_for_music_and_one_frame_for_system_sounds() {
        assert_eq!(prefill_frames(Role::Output, 48_000), 2400);
        assert_eq!(prefill_frames(Role::SoundEffects, 48_000), 1);
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
        let mut waits = 0;
        let e = push_all(&ring, &stereo(5, 1), Duration::from_millis(20), &mut || waits += 1).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert_eq!(ring.queued(), 2);
        assert!(waits > 0, "the caller is told before each sleep");
    }

    #[test]
    fn push_all_reports_the_time_it_slept_for_room() {
        let ring = Ring::new(2);
        let reader = ring.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            while reader.pop().is_some() {}
        });
        let waited = push_all(&ring, &stereo(2, 1), Duration::from_secs(5), &mut || {}).unwrap();
        assert_eq!(waited, Duration::ZERO, "room for all of it: no sleep");
        let waited = push_all(&ring, &stereo(2, 1), Duration::from_secs(5), &mut || {}).unwrap();
        assert!(waited >= Duration::from_millis(5), "{waited:?}");
        t.join().unwrap();
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

    /// The ring of an open session (for counters and the played count).
    fn ring_of(out: &SpeakerOut) -> Arc<Ring> {
        out.0.lock().unwrap_or_else(|p| p.into_inner()).desk.as_ref().expect("the stream is open").ring.clone()
    }

    #[test]
    fn a_prefilled_start_never_underruns() {
        let server = TestServer::start("prefill", "pci");
        let mut out = SpeakerOut::with_socket(&server.path, Role::Output, 48_000, 2);
        // 100 ms in one write: over the 50 ms prefill, so the device starts on it and the first cycle finds data
        write_when_open(&mut out, &stereo(4800, 1000), Duration::from_secs(10)).expect("the stream opens");
        let ring = ring_of(&out);
        let until = Instant::now() + Duration::from_secs(5);
        while ring.played() < 480 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ring.played() >= 480, "the server never ran the IO procedure");
        let st = ring.stats();
        assert_eq!(st.underrun_events, 0, "{st:?}");
        assert_eq!(st.prebuffer_frames, 0, "the IO procedure started on a full prefill: {st:?}");
    }

    #[test]
    fn a_short_write_still_starts_the_device_by_the_deadline() {
        let server = TestServer::start("short", "pci");
        let mut out = SpeakerOut::with_socket(&server.path, Role::Output, 48_000, 2);
        // 10 ms: under the prefill, so the device waits for more, but only START_MAX_WAIT
        write_when_open(&mut out, &stereo(480, 1000), Duration::from_secs(10)).expect("the stream opens");
        let ring = ring_of(&out);
        let until = Instant::now() + Duration::from_secs(5);
        while ring.played() == 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ring.played() > 0, "a sound shorter than the prefill never played");
    }

    #[test]
    fn the_io_procedure_runs_while_the_session_lock_is_held() {
        // The real-time side takes no lock the player thread can hold: it keeps reading the ring while the test holds
        // the session mutex (the writer's, the idle thread's and `played`'s lock).
        let server = TestServer::start("nolock", "pci");
        let mut out = SpeakerOut::with_socket(&server.path, Role::Output, 48_000, 2);
        let chunk = stereo(4800, 1000);
        write_when_open(&mut out, &chunk, Duration::from_secs(10)).expect("the stream opens");
        for _ in 0..4 {
            out.write(&chunk).expect("room in the ring");
        }
        let ring = ring_of(&out);
        let until = Instant::now() + Duration::from_secs(5);
        while ring.played() == 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ring.played() > 0, "the server never ran the IO procedure");
        let guard = out.0.lock().unwrap_or_else(|p| p.into_inner());
        let before = ring.played();
        std::thread::sleep(Duration::from_millis(150));
        let after = ring.played();
        drop(guard);
        assert!(after > before, "the IO procedure stalled while the session was locked ({before} -> {after})");
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
