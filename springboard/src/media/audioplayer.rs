//! The Music app's playback engine: one thread decodes the queue through
//! `audiofile` and writes 48 kHz stereo S16 to an `AudioOut`, gapless (the
//! next song is opened and written as soon as the current one ends; the
//! device never drains between them). It keeps running while the app is
//! closed; the UI reads a `Snapshot` and is woken through `fd()` when the
//! song or the state changes.
//!
//! The samples are written MUTED (zeroed) until `set_muted(false)`: the
//! speaker route is the lead's to open, nothing here ever makes a sound on
//! its own.

use super::aac::{Resampler, OUT_RATE};
use super::timestretch::{self, Stretcher};
use super::speaker::SpeakerOut;
use super::audio_out::AudioOut;
#[cfg(test)]
use super::audio_out::NullOut;
use super::audiofile::{self, AudioDecoder};
use super::queue::{Queue, Repeat};
use super::stream::{self, LiveHandle, LiveStatus, Tuning};
use crate::deildu::http::{Tls, Transport};
use std::ffi::c_int;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Tapping Previous this far into a song restarts it instead.
pub const RESTART_AFTER_MS: f64 = 3000.0;

/// One queued song, or a live stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// the library's id for the song (a station's id for a stream)
    pub id: u64,
    pub path: PathBuf,
    /// the stream's URL; a live track has no file, no length and no seeking
    pub url: Option<String>,
}

impl Track {
    pub fn file(id: u64, path: PathBuf) -> Self {
        Track { id, path, url: None }
    }

    pub fn live(id: u64, url: String) -> Self {
        Track { id, path: PathBuf::new(), url: Some(url) }
    }

    pub fn is_live(&self) -> bool {
        self.url.is_some()
    }
}

/// What the UI shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub track: Option<Track>,
    /// index into the queue's items
    pub index: Option<usize>,
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub queue_len: usize,
    pub volume: f32,
    /// the playback rate (0.5 to 3.0); positions are song time, whatever the rate
    pub rate: f32,
    pub error: Option<String>,
    /// the state of the stream while a live track is current
    pub live: Option<LiveStatus>,
}

struct Shared {
    queue: Queue<Track>,
    playing: bool,
    quit: bool,
    /// bumped whenever the thread must reopen the current song
    generation: u64,
    /// where to start the reopened song, in ms
    start_ms: f64,
    volume: f32,
    /// the playback rate asked for; the thread applies it at its next turn
    rate: f32,
    muted: bool,
    // -- reported by the thread --
    /// the song the device is playing: (index into the queue, track)
    current: Option<(usize, Track)>,
    position_ms: f64,
    duration_ms: Option<f64>,
    error: Option<String>,
    /// the stream of the current live track, shared with the network thread
    live: Option<LiveHandle>,
    /// wakes the UI; also handed to the live handle
    notify: Arc<dyn Fn() + Send + Sync>,
}

pub struct AudioPlayer {
    shared: Arc<Mutex<Shared>>,
    reader: UnixStream,
    worker: Option<JoinHandle<()>>,
}

/// Pokes the UI thread; a full pipe means a wake-up is already pending.
#[derive(Clone)]
struct Waker(Arc<Mutex<UnixStream>>);

impl Waker {
    fn wake(&self) {
        let _ = self.0.lock().unwrap_or_else(|p| p.into_inner()).write(&[1]);
    }
}

impl AudioPlayer {
    /// A player on `out`, idle until `play_queue`.
    pub fn new(out: Box<dyn AudioOut>) -> io::Result<Self> {
        Self::with_transport(out, None, Tuning::default())
    }

    /// As `new`, with the network `transport` live streams use (the HTTPS
    /// client when None) and the stream's timing.
    pub fn with_transport(out: Box<dyn AudioOut>, transport: Option<Arc<dyn Transport>>, tuning: Tuning) -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let waker = Waker(Arc::new(Mutex::new(writer)));
        let notify: Arc<dyn Fn() + Send + Sync> = {
            let waker = waker.clone();
            Arc::new(move || waker.wake())
        };
        let shared = Arc::new(Mutex::new(Shared {
            queue: Queue::new(Vec::new(), 0),
            playing: false,
            quit: false,
            generation: 0,
            start_ms: 0.0,
            volume: 0.6,
            rate: 1.0,
            muted: true,
            position_ms: 0.0,
            duration_ms: None,
            error: None,
            current: None,
            live: None,
            notify,
        }));
        let worker = {
            let shared = shared.clone();
            std::thread::Builder::new().name("music-audio".into()).spawn(move || Engine::new(shared, waker, out, transport, tuning).run())?
        };
        Ok(AudioPlayer { shared, reader, worker: Some(worker) })
    }

    /// A player on the default ALSA PCM (`SPRINGBOARD_PCM` names another plugin). The output
    /// opens the PCM on the first write and closes it again when playback has been idle for a
    /// few seconds (see `speaker`).
    pub fn open_device() -> io::Result<Self> {
        Self::new(Box::new(SpeakerOut::new(OUT_RATE, 2)))
    }

    /// Readable when the song, the state or an error changed.
    pub fn fd(&self) -> c_int {
        self.reader.as_raw_fd()
    }

    /// Clears pending wake-ups.
    pub fn drain(&mut self) {
        let mut sink = [0u8; 64];
        while matches!(self.reader.read(&mut sink), Ok(n) if n > 0) {}
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        Snapshot {
            track: s.current.as_ref().map(|c| c.1.clone()),
            index: s.current.as_ref().map(|c| c.0),
            playing: s.playing,
            position_ms: s.position_ms,
            duration_ms: s.duration_ms,
            shuffle: s.queue.shuffle(),
            repeat: s.queue.repeat(),
            queue_len: s.queue.len(),
            volume: s.volume,
            rate: s.rate,
            error: s.error.clone(),
            live: s.live.as_ref().map(LiveHandle::status),
        }
    }

    /// The queue's songs in list order.
    pub fn queue(&self) -> Vec<Track> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).queue.items().to_vec()
    }

    /// Replaces the queue and starts playing `tracks[start]`; with shuffle
    /// on the rest is reordered with `seed`.
    pub fn play_queue(&mut self, tracks: Vec<Track>, start: usize, shuffle: bool, seed: u64) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        let repeat = s.queue.repeat();
        s.queue = Queue::new(tracks, start);
        s.queue.set_repeat(repeat);
        s.queue.set_shuffle(shuffle, seed);
        s.playing = !s.queue.is_empty();
        s.error = None;
        Self::restart(&mut s, 0.0);
    }

    /// Has the thread reopen the queue's current song at `at_ms`; the
    /// snapshot shows the new song at once.
    fn restart(s: &mut Shared, at_ms: f64) {
        s.generation += 1;
        s.start_ms = at_ms;
        s.position_ms = at_ms;
        s.duration_ms = None;
        s.current = s.queue.current_index().zip(s.queue.current().cloned());
        s.live = s.current.as_ref().filter(|c| c.1.is_live()).map(|_| LiveHandle::new(s.notify.clone()));
    }

    pub fn play(&mut self) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if !s.queue.is_empty() {
            s.playing = true;
        }
    }

    pub fn pause(&mut self) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).playing = false;
    }

    pub fn toggle(&mut self) {
        let playing = self.shared.lock().unwrap_or_else(|p| p.into_inner()).playing;
        if playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Skips to the next song (wrapping at the end).
    pub fn next(&mut self) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if s.queue.next(true) {
            Self::restart(&mut s, 0.0);
        }
    }

    /// Restarts the song when it is past `RESTART_AFTER_MS`, else goes back
    /// one; a live stream has nothing to restart, so it always goes back.
    pub fn prev(&mut self) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if s.queue.is_empty() {
            return;
        }
        let live = s.queue.current().is_some_and(Track::is_live);
        if live || s.position_ms < RESTART_AFTER_MS {
            s.queue.prev();
        }
        Self::restart(&mut s, 0.0);
    }

    /// Jumps to the queue item at `index` (list order).
    pub fn jump_to(&mut self, index: usize) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        s.queue.jump_to(index);
        s.playing = true;
        Self::restart(&mut s, 0.0);
    }

    pub fn seek_ms(&mut self, ms: f64) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if s.queue.is_empty() || s.queue.current().is_some_and(Track::is_live) {
            return;
        }
        let ms = ms.max(0.0).min(s.duration_ms.unwrap_or(f64::MAX));
        Self::restart(&mut s, ms);
    }

    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).queue.set_shuffle(on, seed);
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).queue.set_repeat(r);
    }

    /// Playback speed, 0.5 to 3.0 (NaN is normal speed). The pitch is kept: the audio is time-stretched, not resampled.
    /// The rate stays set across songs until it is changed again.
    pub fn set_rate(&mut self, rate: f32) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).rate = timestretch::clamp_rate(rate);
    }

    /// Software gain 0..=1, applied before the device.
    pub fn set_volume(&mut self, v: f32) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).volume = v.clamp(0.0, 1.0);
    }

    /// Whether the samples reach the device (false) or are zeroed (true,
    /// the default). Only the lead's audio routing turns this off.
    pub fn set_muted(&mut self, muted: bool) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).muted = muted;
    }

    pub fn muted(&self) -> bool {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).muted
    }

    /// Stops the thread; the player is finished afterwards.
    pub fn stop(&mut self) {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).quit = true;
        if let Some(w) = self.worker.take() {
            join_within(w, std::time::Duration::from_millis(300));
        }
    }
}

/// Joins a worker that has been told to quit, but never blocks the caller (the UI thread) for
/// longer than `limit`: a worker stuck in a device call is detached and ends on its own.
pub fn join_within<T>(h: std::thread::JoinHandle<T>, limit: std::time::Duration) -> bool {
    let end = std::time::Instant::now() + limit;
    while !h.is_finished() && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if h.is_finished() {
        let _ = h.join();
        true
    } else {
        false
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Song milliseconds for `frames` device frames played at `rate`: a faster rate covers more of the song in each frame.
fn song_ms(frames: u64, rate: f32) -> f64 {
    frames as f64 * 1000.0 / OUT_RATE as f64 * rate as f64
}

/// The audio thread's state.
struct Engine {
    shared: Arc<Mutex<Shared>>,
    waker: Waker,
    out: Box<dyn AudioOut>,
    /// for live streams; the HTTPS client is built when the first one opens
    transport: Option<Arc<dyn Transport>>,
    tuning: Tuning,
    generation: u64,
    decoder: Option<Box<dyn AudioDecoder>>,
    resampler: Resampler,
    /// the time-stretch for rates other than 1; fed only while `applied_rate` is not 1
    stretch: Stretcher,
    /// the rate the device output is at (the stretcher's rate), which may lag the request by one turn
    applied_rate: f32,
    /// the stretched block being written (reused between blocks)
    stretched: Vec<i16>,
    /// frames written to the device since its last discard
    written: u64,
    /// `written` when the song now playing started, and its start time
    song_start: (u64, f64),
    /// songs queued in the device behind the one playing: (first frame, duration, queue index, track)
    pending: std::collections::VecDeque<(u64, Option<f64>, usize, Track)>,
    paused: bool,
    /// `out.played()` just before the last pause, to see whether the device kept its queue.
    played_at_pause: u64,
    /// ramp the next block up from silence (after a flush)
    fade_in: bool,
    /// the decoder reached the end and no song follows
    drained: bool,
    samples: Vec<i16>,
    pcm: Vec<i16>,
    /// the last once-a-second trace line
    traced: std::time::Instant,
}

/// Length of the ramp after a flush: 10 ms at 48 kHz.
const FADE_FRAMES: usize = 480;

/// Ramps interleaved stereo frames linearly from silence over their first `frames` frames.
fn fade_in(pcm: &mut [i16], frames: usize) {
    let n = frames.min(pcm.len() / 2).max(1);
    for (i, pair) in pcm.chunks_mut(2).take(n).enumerate() {
        for s in pair {
            *s = (*s as i32 * i as i32 / n as i32) as i16;
        }
    }
}

impl Engine {
    fn new(shared: Arc<Mutex<Shared>>, waker: Waker, out: Box<dyn AudioOut>, transport: Option<Arc<dyn Transport>>, tuning: Tuning) -> Self {
        Engine { shared, waker, out, transport, tuning, generation: 0, decoder: None, resampler: Resampler::new(OUT_RATE, 2), stretch: Stretcher::new(1.0), applied_rate: 1.0, stretched: Vec::new(), written: 0, song_start: (0, 0.0), pending: Default::default(), paused: false, played_at_pause: 0, fade_in: true, drained: false, samples: Vec::new(), pcm: Vec::new(), traced: std::time::Instant::now() }
    }

    /// Opens the current song at `start_ms`; None (with the error noted)
    /// when it does not play.
    fn open_current(&mut self, start_ms: f64) -> Option<Box<dyn AudioDecoder>> {
        let track = self.shared.lock().unwrap_or_else(|p| p.into_inner()).queue.current().cloned()?;
        let opened = match &track.url {
            Some(url) => self.open_stream(url),
            None => audiofile::open(&track.path).map_err(|e| format!("{}: {e}", track.path.display())),
        };
        let mut d = match opened {
            Ok(d) => d,
            Err(e) => {
                self.shared.lock().unwrap_or_else(|p| p.into_inner()).error = Some(e);
                return None;
            }
        };
        if start_ms > 0.0 && !track.is_live() {
            let _ = d.seek_ms(start_ms);
        }
        self.resampler = Resampler::new(d.rate(), d.channels());
        Some(d)
    }

    /// Connects to a live stream; blocks until audio arrives, but gives up
    /// as soon as the song is changed or the player stopped.
    fn open_stream(&mut self, url: &str) -> Result<Box<dyn AudioDecoder>, String> {
        let live = self.shared.lock().unwrap_or_else(|p| p.into_inner()).live.clone().ok_or("no stream state")?;
        let transport = self.transport.get_or_insert_with(|| Arc::new(Tls::new())).clone();
        let (shared, generation) = (self.shared.clone(), self.generation);
        let abort = Box::new(move || {
            let s = shared.lock().unwrap_or_else(|p| p.into_inner());
            s.quit || s.generation != generation
        });
        stream::open(url, transport, live, abort, self.tuning).map_err(|e| format!("{url}: {e}"))
    }

    fn reopen(&mut self, start_ms: f64) {
        // queued audio plays out over silence, then the ring is dropped: no click, nothing stale
        self.out.flush();
        self.fade_in = true;
        self.written = 0;
        self.pending.clear();
        self.drained = false;
        self.stretch.reset();
        self.decoder = self.open_current(start_ms);
        self.song_start = (0, start_ms);
        let duration = self.decoder.as_ref().and_then(|d| d.duration_ms());
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        s.duration_ms = duration;
        s.position_ms = start_ms;
        s.current = s.queue.current_index().zip(s.queue.current().cloned());
        if self.decoder.is_none() {
            s.playing = false;
        }
        drop(s);
        self.waker.wake();
    }

    /// Applies a rate the player was asked for. The position mapping is re-anchored at the device's current frame, so
    /// the reported song position stays continuous across the change.
    fn sync_rate(&mut self, want: f32) {
        let played = self.out.played();
        let (first, start) = self.song_start;
        let pos = start + song_ms(played.saturating_sub(first), self.applied_rate);
        self.song_start = (played, pos);
        self.applied_rate = want;
        self.stretch.set_rate(want);
        if want == 1.0 {
            // the stretcher is bypassed at 1: drop what it still holds so a later stretch starts clean
            self.stretch.reset();
        }
    }

    /// Updates the reported position; switches the reported song when the
    /// device has reached a queued one.
    fn report(&mut self) {
        let played = self.out.played();
        while self.pending.front().is_some_and(|p| played >= p.0) {
            let (first, duration, index, track) = self.pending.pop_front().unwrap();
            self.song_start = (first, 0.0);
            let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            s.duration_ms = duration;
            s.current = Some((index, track));
            drop(s);
            self.waker.wake();
        }
        let (first, start) = self.song_start;
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if !s.playing {
            // a pause landed while a write was in flight: the position stays put
            return;
        }
        let pos = start + song_ms(played.saturating_sub(first), self.applied_rate);
        s.position_ms = match s.duration_ms {
            Some(d) => pos.min(d),
            None => pos,
        };
    }

    fn run(mut self) {
        crate::gfx::affinity::audio_thread();
        // no deep C-state exit while a song plays (see bare::pmqos); dropped on pause, stop and quit
        let mut latency: Option<crate::bare::pmqos::CpuDmaLatency> = None;
        loop {
            let (quit, generation, playing, start_ms, rate) = {
                let s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
                (s.quit, s.generation, s.playing, s.start_ms, s.rate)
            };
            if quit {
                return;
            }
            if rate != self.applied_rate {
                self.sync_rate(rate);
            }
            if playing && !self.paused {
                if latency.is_none() {
                    latency = crate::bare::pmqos::CpuDmaLatency::hold();
                }
            } else {
                latency = None;
            }
            if generation != self.generation {
                self.generation = generation;
                self.reopen(start_ms);
                continue;
            }
            if playing == self.paused {
                self.paused = !playing;
                if self.paused {
                    // taken before the pause: a device that drops its queue restarts at 0
                    self.played_at_pause = self.out.played();
                }
                self.out.pause(self.paused);
                if !self.paused && self.out.played() < self.played_at_pause {
                    // the device dropped its queue on pause: resume from where it stopped
                    let at = self.shared.lock().unwrap_or_else(|p| p.into_inner()).position_ms;
                    self.reopen(at);
                    continue;
                }
                self.waker.wake();
            }
            if self.paused {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if self.drained {
                self.report();
                if self.out.played() >= self.written {
                    // the last song ended: rest paused on the first one
                    let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
                    s.playing = false;
                    s.queue.next(false);
                    AudioPlayer::restart(&mut s, 0.0);
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if self.decoder.is_none() {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let more = {
                let d = self.decoder.as_mut().unwrap();
                match d.next(&mut self.samples) {
                    Ok(more) => more,
                    Err(e) => {
                        self.shared.lock().unwrap_or_else(|p| p.into_inner()).error = Some(e.to_string());
                        false
                    }
                }
            };
            if !more {
                if self.superseded() {
                    // a blocked stream gave up because the song changed
                    self.decoder = None;
                    continue;
                }
                self.advance();
                continue;
            }
            self.write_samples();
            self.report();
        }
    }

    /// Whether a newer request (or quit) arrived since the song was opened.
    fn superseded(&self) -> bool {
        let s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        s.quit || s.generation != self.generation
    }

    /// The song ended: open what follows without draining the device. A
    /// live stream that ends has failed; the next station is not its
    /// successor.
    fn advance(&mut self) {
        let next = {
            let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            if s.queue.current().is_some_and(Track::is_live) {
                None
            } else if s.queue.following(false).is_some() {
                s.queue.next(false);
                s.queue.current_index().zip(s.queue.current().cloned())
            } else {
                None
            }
        };
        self.decoder = None;
        // the held-back tail of the song that ended is not the next song's start
        self.stretch.reset();
        if let Some((index, track)) = next {
            self.decoder = self.open_current(0.0);
            let duration = self.decoder.as_ref().and_then(|d| d.duration_ms());
            self.pending.push_back((self.written, duration, index, track));
            if self.decoder.is_none() {
                // an unplayable song: stop there
                self.drained = true;
            }
        } else {
            self.drained = true;
        }
    }

    fn write_samples(&mut self) {
        let (volume, muted) = {
            let s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            (s.volume, s.muted)
        };
        let d = self.decoder.as_ref().unwrap();
        self.resampler.retune(d.rate(), d.channels());
        self.pcm.clear();
        self.resampler.convert(&self.samples, &mut self.pcm);
        if self.applied_rate != 1.0 {
            self.stretched.clear();
            self.stretch.push(&self.pcm, &mut self.stretched);
            std::mem::swap(&mut self.pcm, &mut self.stretched);
            if self.pcm.is_empty() {
                // the stretcher is still filling its first window
                return;
            }
        }
        if self.traced.elapsed() >= Duration::from_secs(1) {
            self.traced = std::time::Instant::now();
            let peak = |v: &[i16]| v.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            eprintln!(
                "audioplayer: trace decoded {} samples peak {} @ {} Hz x{} -> {} frames peak {}, volume {:.2} muted {muted}",
                self.samples.len(), peak(&self.samples), d.rate(), d.channels(), self.pcm.len() / 2, peak(&self.pcm), volume
            );
        }
        if muted {
            self.pcm.iter_mut().for_each(|s| *s = 0);
        } else {
            // the shell's curve (media::volume) on this player's own volume, applied once
            super::volume::scale(&mut self.pcm, super::volume::Volume::new(volume).linear());
        }
        if std::mem::take(&mut self.fade_in) {
            fade_in(&mut self.pcm, FADE_FRAMES);
        }
        match super::audio_out::write_with_retry(&mut *self.out, &self.pcm, &super::audio_out::WRITE_RETRY_BACKOFF) {
            Ok(()) => self.written += self.pcm.len() as u64 / 2,
            Err(e) => {
                // the device is gone: stop here instead of racing through the queue
                let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
                s.playing = false;
                s.error = Some(format!("audio output: {e}"));
                drop(s);
                self.waker.wake();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::audiofile::tests::fixture;
    use super::super::stream::fake::{icecast, mp3_frames, Local, ICY};
    use super::super::stream::Phase;
    use super::*;
    use std::time::Instant;

    #[test]
    fn player_volume_scales_samples_by_the_volume_curve() {
        let mut pcm = vec![10000i16, -10000];
        let g = super::super::volume::Volume::new(0.5).linear();
        super::super::volume::scale(&mut pcm, g);
        assert!((pcm[0] as f32 - 10000.0 * g).abs() <= 1.0 && pcm[0] == -pcm[1]);
        assert!(g > 0.0 && g < 1.0);
    }

    #[test]
    fn set_rate_clamps_and_the_snapshot_reports_it() {
        let mut p = AudioPlayer::new(Box::new(NullOut::new())).unwrap();
        assert_eq!(p.snapshot().rate, 1.0, "normal speed until asked");
        p.set_rate(4.0);
        assert_eq!(p.snapshot().rate, 2.0);
        p.set_rate(0.25);
        assert_eq!(p.snapshot().rate, 0.5);
        p.set_rate(f32::NAN);
        assert_eq!(p.snapshot().rate, 1.0);
        p.set_rate(1.5);
        assert_eq!(p.snapshot().rate, 1.5);
    }

    #[test]
    fn song_time_runs_faster_than_the_device_at_a_higher_rate() {
        // a second of device time is two seconds of song at 2x and half a second at 0.5x
        assert_eq!(song_ms(OUT_RATE as u64, 1.0), 1000.0);
        assert_eq!(song_ms(OUT_RATE as u64, 2.0), 2000.0);
        assert_eq!(song_ms(OUT_RATE as u64 * 2, 0.5), 1000.0);
    }

    #[test]
    fn fade_in_ramps_from_silence_and_leaves_the_rest() {
        let mut pcm = vec![1000i16; 12];
        fade_in(&mut pcm, 4);
        assert_eq!(&pcm[..2], &[0, 0]);
        assert!(pcm[2] < pcm[4] && pcm[4] < pcm[6] && pcm[6] < 1000);
        assert_eq!(&pcm[8..], &[1000; 4]);
        let mut short = vec![500i16; 2];
        fade_in(&mut short, 480);
        assert_eq!(short, [0, 0]);
    }

    fn tracks() -> Vec<Track> {
        vec![Track::file(1, fixture("tone_id3v23.mp3")), Track::file(2, fixture("tone.m4a")), Track::file(3, fixture("tone_id3v24.mp3"))]
    }

    fn player() -> AudioPlayer {
        AudioPlayer::new(Box::new(NullOut::new())).unwrap()
    }

    /// Waits (polling the wake fd) until `done` or the timeout.
    fn wait(p: &mut AudioPlayer, ms: u64, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let start = Instant::now();
        loop {
            let s = p.snapshot();
            if done(&s) || start.elapsed().as_millis() > ms as u128 {
                return s;
            }
            crate::bare::sys::wait(&[p.fd()], 20);
            p.drain();
        }
    }

    #[test]
    fn plays_the_queue_gaplessly_to_the_end_and_rests_on_the_first_song() {
        let mut p = player();
        assert!(p.muted(), "muted until the lead routes the speaker");
        let s = p.snapshot();
        assert!(s.track.is_none() && !s.playing && s.queue_len == 0);
        p.play_queue(tracks(), 0, false, 1);
        let s = wait(&mut p, 1000, |s| s.duration_ms.is_some());
        assert_eq!(s.index, Some(0));
        assert!(s.playing);
        assert!((s.duration_ms.unwrap() - 1200.0).abs() < 60.0, "{:?}", s.duration_ms);
        let s = wait(&mut p, 8000, |s| s.index == Some(1));
        assert_eq!(s.track.map(|t| t.id), Some(2), "moved to the m4a");
        // NullOut plays in real time, so a starved test thread can see the song late: only
        // require that the position belongs to the new song, not that it is still near zero.
        assert!(s.position_ms <= s.duration_ms.unwrap_or(f64::MAX), "its position is inside the song: {}", s.position_ms);
        let s = wait(&mut p, 8000, |s| s.index == Some(2));
        assert_eq!(s.index, Some(2), "{s:?}");
        let s = wait(&mut p, 8000, |s| !s.playing);
        assert!(!s.playing, "stops after the last song");
        assert_eq!(s.index, Some(0), "rests on the first song");
        assert_eq!(s.position_ms, 0.0);
        assert!(s.error.is_none(), "{:?}", s.error);
    }

    #[test]
    fn pause_freezes_seek_moves_and_prev_next_navigate() {
        let mut p = player();
        p.play_queue(tracks(), 1, false, 1);
        let s = wait(&mut p, 1000, |s| s.position_ms > 150.0);
        assert_eq!(s.index, Some(1));
        p.pause();
        let s = wait(&mut p, 300, |s| !s.playing);
        let at = s.position_ms;
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(p.snapshot().position_ms, at, "frozen while paused");
        p.seek_ms(900.0);
        let s = wait(&mut p, 500, |s| s.position_ms >= 900.0 && s.duration_ms.is_some());
        assert!((s.position_ms - 900.0).abs() < 50.0, "seeked while paused: {}", s.position_ms);
        p.toggle();
        let s = wait(&mut p, 1000, |s| s.position_ms > 1000.0);
        assert!(s.playing && s.position_ms > 1000.0);
        p.prev();
        let s = wait(&mut p, 500, |s| s.index == Some(0));
        assert_eq!(s.index, Some(0), "early in the song, prev goes back a song");
        p.next();
        p.next();
        let s = wait(&mut p, 500, |s| s.index == Some(2));
        assert_eq!(s.index, Some(2));
        p.next();
        let s = wait(&mut p, 500, |s| s.index == Some(0));
        assert_eq!(s.index, Some(0), "next wraps");
        p.seek_ms(1100.0);
        wait(&mut p, 500, |s| s.position_ms >= 1100.0);
        p.prev();
        let s = wait(&mut p, 500, |s| s.position_ms < 100.0);
        assert_eq!(s.index, Some(0), "late in the song, prev restarts it");
        p.stop();
    }

    #[test]
    fn resume_after_a_device_that_drops_its_queue_keeps_the_position() {
        let mut p = AudioPlayer::new(Box::new(NullOut::dropping_on_pause())).unwrap();
        p.play_queue(tracks(), 1, false, 1);
        wait(&mut p, 1000, |s| s.position_ms > 300.0);
        p.pause();
        let at = wait(&mut p, 300, |s| !s.playing).position_ms;
        p.play();
        let s = wait(&mut p, 1000, |s| s.playing && s.position_ms > at + 100.0);
        assert!(s.position_ms > at, "position went back: {} < {at}", s.position_ms);
        assert!(s.error.is_none(), "{:?}", s.error);
        p.stop();
    }

    #[test]
    fn last_song_still_drains_after_a_pause_on_a_dropping_device() {
        let mut p = AudioPlayer::new(Box::new(NullOut::dropping_on_pause())).unwrap();
        p.play_queue(tracks()[..1].to_vec(), 0, false, 1);
        wait(&mut p, 1000, |s| s.position_ms > 200.0);
        p.pause();
        wait(&mut p, 300, |s| !s.playing);
        p.play();
        let s = wait(&mut p, 5000, |s| !s.playing);
        assert!(!s.playing, "queue ends and rests: {s:?}");
        assert_eq!(s.position_ms, 0.0);
    }

    #[test]
    fn repeat_one_and_shuffle_are_reflected() {
        let mut p = player();
        p.set_repeat(Repeat::One);
        p.play_queue(tracks(), 2, false, 1);
        let s = wait(&mut p, 2500, |s| s.position_ms > 1100.0);
        assert_eq!((s.index, s.repeat), (Some(2), Repeat::One));
        let s = wait(&mut p, 1500, |s| s.position_ms < 500.0);
        assert_eq!(s.index, Some(2), "repeats the same song");
        assert!(s.playing);
        p.set_shuffle(true, 7);
        assert!(p.snapshot().shuffle);
        p.set_volume(2.0);
        assert_eq!(p.snapshot().volume, 1.0);
        p.set_muted(false);
        assert!(!p.muted());
        p.set_muted(true);
        assert_eq!(p.queue().len(), 3);
        p.jump_to(1);
        let s = wait(&mut p, 500, |s| s.index == Some(1));
        assert_eq!(s.index, Some(1));
    }

    #[test]
    fn unplayable_song_reports_an_error_and_stops() {
        let mut p = player();
        p.play_queue(vec![Track::file(9, fixture("missing.mp3"))], 0, false, 1);
        let s = wait(&mut p, 1000, |s| s.error.is_some());
        assert!(s.error.as_deref().unwrap_or("").contains("missing.mp3"), "{:?}", s.error);
        assert!(!s.playing);
        p.play();
        assert!(!p.snapshot().playing || p.snapshot().queue_len == 1);
    }

    const FAST: Tuning = Tuning { prebuffer: 4096, backoff_first_ms: 10, backoff_max_ms: 40 };

    fn radio_player(routes: Vec<(&'static str, u16)>) -> AudioPlayer {
        AudioPlayer::with_transport(Box::new(NullOut::new()), Some(Arc::new(Local(routes))), FAST).unwrap()
    }

    fn station(id: u64, host: &str) -> Track {
        Track::live(id, format!("http://{host}/live"))
    }

    #[test]
    fn plays_live_stations_with_titles_and_no_length_and_steps_through_them() {
        let (a, _) = icecast(ICY, mp3_frames(), 1000, &["Artist - A"], None);
        let (b, _) = icecast(ICY, mp3_frames(), 1000, &["Artist - B"], None);
        let mut p = radio_player(vec![("a.test", a), ("b.test", b)]);
        p.play_queue(vec![station(1, "a.test"), station(2, "b.test")], 1, false, 1);
        let s = wait(&mut p, 1, |s| s.live.is_some());
        assert!(s.live.is_some_and(|l| l.phase == Phase::Buffering), "the state shows at once, before any audio");
        let s = wait(&mut p, 5000, |s| s.live.as_ref().is_some_and(|l| l.phase == Phase::Live && l.title.is_some()));
        let live = s.live.clone().expect("a live snapshot");
        assert_eq!((live.phase, live.title.as_deref()), (Phase::Live, Some("Artist - B")), "{s:?}");
        assert!(s.playing && s.duration_ms.is_none() && s.track.as_ref().is_some_and(Track::is_live) && s.error.is_none(), "{s:?}");
        p.seek_ms(500.0);
        let s = wait(&mut p, 3400, |s| s.position_ms > 3200.0);
        assert!(s.position_ms > 3200.0 && s.index == Some(1), "seeking a stream does nothing: {s:?}");
        p.prev();
        let s = wait(&mut p, 5000, |s| s.live.as_ref().is_some_and(|l| l.phase == Phase::Live && l.title.as_deref() == Some("Artist - A")));
        assert_eq!(s.index, Some(0), "Previous steps to the other station even late in the stream: {s:?}");
        p.next();
        let s = wait(&mut p, 5000, |s| s.index == Some(1) && s.live.as_ref().is_some_and(|l| l.title.as_deref() == Some("Artist - B")));
        assert_eq!(s.index, Some(1));
        p.pause();
        assert!(!wait(&mut p, 500, |s| !s.playing).playing);
    }

    #[test]
    fn a_dead_station_shows_reconnecting_and_can_be_left_for_the_next_one() {
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (ok, _) = icecast(ICY, mp3_frames(), 0, &[], None);
        let mut p = radio_player(vec![("dead.test", dead), ("ok.test", ok)]);
        p.play_queue(vec![station(1, "dead.test"), station(2, "ok.test")], 0, false, 1);
        let s = wait(&mut p, 5000, |s| s.live.as_ref().is_some_and(|l| matches!(l.phase, Phase::Reconnecting(n) if n >= 2)));
        let live = s.live.expect("a live snapshot");
        assert!(matches!(live.phase, Phase::Reconnecting(n) if n >= 2) && live.error.is_some(), "{live:?}");
        assert!(s.playing && s.index == Some(0), "it keeps trying instead of skipping on");
        p.next();
        let s = wait(&mut p, 5000, |s| s.live.as_ref().is_some_and(|l| l.phase == Phase::Live));
        assert_eq!((s.index, s.live.map(|l| l.phase)), (Some(1), Some(Phase::Live)), "a blocked connect gives way to Next");
    }

    #[test]
    fn a_stream_that_is_not_mp3_stops_with_an_error() {
        let (port, _) = icecast("HTTP/1.0 200 OK\r\n\r\n", b"<html>not audio, only a page of text to probe</html>".repeat(300), 0, &[], None);
        let mut p = radio_player(vec![("web.test", port)]);
        p.play_queue(vec![station(1, "web.test")], 0, false, 1);
        let s = wait(&mut p, 5000, |s| s.error.is_some());
        assert!(s.error.as_deref().unwrap_or("").contains("web.test"), "{:?}", s.error);
        assert!(!s.playing);
    }
}
