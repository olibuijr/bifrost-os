//! The player: demux → decoders → an audio thread (clock master) and a
//! video thread that produces display-sized RGBA frames for the UI.
//!
//! The UI polls `fd()` and calls `take_frame()` / `state()` when it is
//! readable. Both worker threads share the demuxer under a mutex and poll
//! a small command block; the audio thread blocks in the device write,
//! the video thread sleeps until the next frame's presentation time.
//! When the file runs out of bytes a thread parks in `Buffering` and
//! retries every 50 ms until the streamer has written the range.

use super::audio_out::AudioOut;
#[cfg(test)]
use super::audio_out::NullOut;
use super::mp4::{self, Demuxer, Kind};
use super::source::{FileSource, Source};
use super::{aac, convert, Codec, Error, Picture, Pixels, VideoDecoder};
use crate::gfx::Image;
use std::ffi::c_int;
use std::io;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Playing,
    Paused,
    /// Waiting for `need` = (offset, len) to appear in the file.
    Buffering { need: (u64, u64) },
    Ended,
    Error(String),
}

/// Where the viewport defaults to until the UI reports the video area.
const DEFAULT_VIEWPORT: (u32, u32) = (1920, 1080);
/// Threads for the NV12 → RGBA conversion (the rest is the decoder's).
const CONVERT_THREADS: usize = 3;
/// Decoded frames held ahead of the clock.
const LOOKAHEAD: usize = 3;
/// A frame this late is dropped rather than shown.
const LATE_MS: f64 = 60.0;
/// How long to wait before re-reading a missing range.
const RETRY: Duration = Duration::from_millis(50);

/// Media time, driven by the audio device while audio plays and by the
/// wall clock otherwise.
struct Clock {
    media_ms: f64,
    anchor: Instant,
    running: bool,
}

impl Clock {
    fn now(&self) -> f64 {
        if self.running {
            self.media_ms + self.anchor.elapsed().as_secs_f64() * 1000.0
        } else {
            self.media_ms
        }
    }

    fn set(&mut self, media_ms: f64, running: bool) {
        self.media_ms = media_ms;
        self.anchor = Instant::now();
        self.running = running;
    }
}

struct Shared {
    playing: bool,
    quit: bool,
    /// bumped by every seek; threads reset when they see a new value
    generation: u64,
    /// a seek nobody has applied to the demuxer yet
    seek_pending: Option<f64>,
    /// where the last seek went; frames before it are not shown
    seek_target: f64,
    duration_ms: Option<f64>,
    video_wait: Option<(u64, u64)>,
    audio_wait: Option<(u64, u64)>,
    video_done: bool,
    audio_done: bool,
    has_audio: bool,
    error: Option<String>,
    frame: Option<(Picture, f64)>,
    viewport: (u32, u32),
    /// the UI draws pictures on the GPU: leave them in the decoder's buffers
    zero_copy: bool,
    clock: Clock,
}

type DecoderFactory = Box<dyn FnOnce(Codec) -> io::Result<Box<dyn VideoDecoder>> + Send>;

pub struct Player {
    shared: Arc<Mutex<Shared>>,
    pipe: [c_int; 2],
    worker: Option<JoinHandle<()>>,
    duration_ms: Option<f64>,
}

extern "C" {
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
    fn write(fd: c_int, buf: *const std::ffi::c_void, n: usize) -> isize;
}

/// Wakes the UI thread: one byte down the pipe (ignored when it is full).
fn notify(fd: c_int) {
    // SAFETY: writes one byte from a valid buffer to a pipe we own.
    unsafe { write(fd, [1u8].as_ptr() as *const _, 1) };
}

impl Player {
    /// Opens `path` with the system decoder and the default ALSA PCM
    /// (`SPRINGBOARD_PCM` names another ALSA plugin; the speaker session
    /// opens on the first sound, and without one video plays against the wall clock). Starts the threads.
    pub fn open(path: &std::path::Path) -> io::Result<Player> {
        let source = Box::new(FileSource::open(path)?);
        // the same lazy session Music uses: opens the PCM on the first write
        let audio: Box<dyn AudioOut> = Box::new(super::speaker::SpeakerOut::new(aac::OUT_RATE, 2));
        Self::start(source, Box::new(super::default_decoder), audio)
    }

    /// Like `open`, with a caller-supplied video decoder.
    pub fn open_with(path: &std::path::Path, decoder: Box<dyn VideoDecoder>) -> io::Result<Player> {
        let source = Box::new(FileSource::open(path)?);
        let audio: Box<dyn AudioOut> = Box::new(super::speaker::SpeakerOut::new(aac::OUT_RATE, 2));
        Self::start(source, Box::new(move |_| Ok(decoder)), audio)
    }

    /// Everything injected: the byte source, the video decoder and the
    /// audio output (tests use a memory source and a silent output).
    pub fn open_parts(source: Box<dyn Source>, decoder: Box<dyn VideoDecoder>, audio: Box<dyn AudioOut>) -> io::Result<Player> {
        Self::start(source, Box::new(move |_| Ok(decoder)), audio)
    }

    fn start(source: Box<dyn Source>, decoder: DecoderFactory, audio: Box<dyn AudioOut>) -> io::Result<Player> {
        let mut pipe = [0; 2];
        // SAFETY: pipe2 fills the two-element array; O_NONBLOCK | O_CLOEXEC.
        if unsafe { pipe2(pipe.as_mut_ptr(), 0o4000 | 0o2000000) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let shared = Arc::new(Mutex::new(Shared {
            playing: true,
            quit: false,
            generation: 0,
            seek_pending: None,
            seek_target: 0.0,
            duration_ms: None,
            video_wait: None,
            audio_wait: None,
            video_done: false,
            audio_done: false,
            has_audio: false,
            error: None,
            frame: None,
            viewport: DEFAULT_VIEWPORT,
            zero_copy: false,
            clock: Clock { media_ms: 0.0, anchor: Instant::now(), running: false },
        }));
        let worker = {
            let shared = shared.clone();
            let notify_fd = pipe[1];
            std::thread::Builder::new().name("media-video".into()).spawn(move || {
                if let Err(e) = run(shared.clone(), notify_fd, source, decoder, audio) {
                    shared.lock().unwrap().error = Some(e);
                    notify(notify_fd);
                }
            })?
        };
        Ok(Player { shared, pipe, worker: Some(worker), duration_ms: None })
    }

    pub fn play(&mut self) {
        let mut s = self.shared.lock().unwrap();
        if !s.playing {
            s.playing = true;
            let t = s.clock.now();
            s.clock.set(t, true);
        }
    }

    pub fn pause(&mut self) {
        let mut s = self.shared.lock().unwrap();
        if s.playing {
            s.playing = false;
            let t = s.clock.now();
            s.clock.set(t, false);
        }
    }

    /// Stops the threads; the player is finished afterwards.
    /// Returns false when the worker was detached because it did not exit in time.
    pub fn stop(&mut self) -> bool {
        self.shared.lock().unwrap().quit = true;
        match self.worker.take() {
            Some(w) => super::audioplayer::join_within(w, std::time::Duration::from_millis(300)),
            None => true,
        }
    }

    /// Jumps to the keyframe at or before `ms`.
    pub fn seek_ms(&mut self, ms: f64) {
        let mut s = self.shared.lock().unwrap();
        let ms = ms.max(0.0).min(s.duration_ms.unwrap_or(f64::MAX));
        s.seek_pending = Some(ms);
        s.seek_target = ms;
        s.generation += 1;
        s.video_done = false;
        s.audio_done = false;
        s.frame = None;
        let playing = s.playing;
        s.clock.set(ms, playing);
    }

    pub fn position_ms(&self) -> f64 {
        let s = self.shared.lock().unwrap();
        let t = s.seek_pending.unwrap_or_else(|| s.clock.now());
        match s.duration_ms {
            Some(d) => t.min(d),
            None => t,
        }
    }

    pub fn duration_ms(&self) -> Option<f64> {
        self.duration_ms.or_else(|| self.shared.lock().unwrap().duration_ms)
    }

    pub fn state(&self) -> State {
        let s = self.shared.lock().unwrap();
        if let Some(e) = &s.error {
            return State::Error(e.clone());
        }
        if self.worker.is_none() {
            return State::Ended;
        }
        if s.video_done && (!s.has_audio || s.audio_done) && s.frame.is_none() {
            return State::Ended;
        }
        if let Some(need) = s.video_wait.or(s.audio_wait) {
            return State::Buffering { need };
        }
        if s.playing {
            State::Playing
        } else {
            State::Paused
        }
    }

    /// The latest decoded frame, at most once per frame; drains the pipe.
    pub fn take_frame(&mut self) -> Option<Image> {
        let viewport = self.shared.lock().unwrap().viewport;
        self.take_picture().map(|p| p.into_rgba(viewport))
    }

    /// Like `take_frame`, but a zero-copy picture stays in the decoder's buffer.
    pub fn take_picture(&mut self) -> Option<Picture> {
        let mut buf = [0u8; 64];
        // SAFETY: non-blocking read into a local buffer from our own pipe.
        while unsafe { crate::bare::sys::read(self.pipe[0], buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {}
        self.shared.lock().unwrap().frame.take().map(|(p, _)| p)
    }

    /// Asks for pictures left in the decoder's buffers (`Picture::Gpu`) when the decoder can;
    /// off, they are converted to RGBA at the viewport size on the video thread.
    pub fn set_zero_copy(&mut self, on: bool) {
        self.shared.lock().unwrap().zero_copy = on;
    }

    /// Readable when a new frame or state is available.
    pub fn fd(&self) -> c_int {
        self.pipe[0]
    }

    /// Hands the latest frame to a `Sink`, if there is a new one.
    pub fn present_to(&mut self, sink: &mut dyn super::Sink) -> bool {
        let mut buf = [0u8; 64];
        // SAFETY: non-blocking read into a local buffer from our own pipe.
        while unsafe { crate::bare::sys::read(self.pipe[0], buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {}
        let taken = {
            let mut s = self.shared.lock().unwrap();
            s.frame.take().map(|f| (f, s.viewport))
        };
        match taken {
            Some(((p, pts), viewport)) => {
                sink.present(&p.into_rgba(viewport), pts);
                true
            }
            None => false,
        }
    }

    /// The box frames are scaled to fit (default: the 1200x1920 panel).
    pub fn set_viewport(&mut self, w: u32, h: u32) {
        self.shared.lock().unwrap().viewport = (w.max(2), h.max(2));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // A worker that did not exit in time may still use the pipe: leak the fds rather than
        // let them be reused under it.
        if self.stop() {
            for fd in self.pipe {
                // SAFETY: both ends are ours and unused after this.
                unsafe { crate::bare::sys::close(fd) };
            }
        }
    }
}

/// The demuxer and its source, shared by both threads.
type Demux = Arc<Mutex<(Demuxer, Box<dyn Source>)>>;

/// Opens the demuxer (waiting for the file to grow if needed), starts the
/// audio thread and runs the video loop; the worker body.
fn run(shared: Arc<Mutex<Shared>>, notify_fd: c_int, mut source: Box<dyn Source>, factory: DecoderFactory, audio: Box<dyn AudioOut>) -> Result<(), String> {
    let demuxer = loop {
        match Demuxer::open(&mut *source) {
            Ok(d) => break d,
            Err(Error::NeedData { offset, len }) => {
                shared.lock().unwrap().video_wait = Some((offset, len));
                notify(notify_fd);
                std::thread::sleep(RETRY);
                if shared.lock().unwrap().quit {
                    return Ok(());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let duration = demuxer.duration_ms();
    let video = demuxer.video().map(|t| t.codec.clone());
    let has_audio = demuxer.audio().is_some();
    {
        let mut s = shared.lock().unwrap();
        s.duration_ms = duration;
        s.has_audio = has_audio;
        s.video_wait = None;
        s.video_done = video.is_none();
        s.audio_done = !has_audio;
        let playing = s.playing;
        s.clock.set(0.0, playing);
    }
    notify(notify_fd);
    let demux: Demux = Arc::new(Mutex::new((demuxer, source)));
    let audio_thread = if has_audio {
        let (shared, demux) = (shared.clone(), demux.clone());
        std::thread::Builder::new().name("media-audio".into()).spawn(move || audio_loop(shared, notify_fd, demux, audio)).ok()
    } else {
        None
    };
    let result = match video {
        Some(codec) => video_loop(&shared, notify_fd, &demux, codec, factory),
        None => Ok(()),
    };
    if result.is_err() {
        shared.lock().unwrap().quit = true;
    }
    if let Some(t) = audio_thread {
        let _ = t.join();
    }
    result
}

/// The next sample of `kind`, or None when a seek was requested after
/// `generation` (the caller resets first, so the keyframe is not lost).
/// Checked under the demuxer lock, which every seek also takes.
fn fetch(shared: &Mutex<Shared>, demux: &Demux, kind: Kind, generation: u64) -> Option<Result<Option<mp4::Sample>, Error>> {
    let mut d = demux.lock().unwrap();
    if shared.lock().unwrap().generation != generation {
        return None;
    }
    let (demuxer, source) = &mut *d;
    Some(demuxer.next(kind, &mut **source))
}

/// Sleeps until `pts_ms` on the shared clock, or until a seek, pause or
/// quit intervenes; true if the frame is still the right one to show.
fn wait_until(shared: &Mutex<Shared>, pts_ms: f64, generation: u64) -> bool {
    loop {
        let (now, playing) = {
            let s = shared.lock().unwrap();
            if s.quit || s.generation != generation {
                return false;
            }
            (s.clock.now(), s.playing)
        };
        if playing && now >= pts_ms - 1.0 {
            return true;
        }
        let ahead = if playing { (pts_ms - now).clamp(1.0, 5.0) } else { 10.0 };
        std::thread::sleep(Duration::from_secs_f64(ahead / 1000.0));
    }
}

/// Applies a pending seek to the demuxer; false while its data is missing.
fn apply_seek(shared: &Mutex<Shared>, demux: &Demux, wait: fn(&mut Shared) -> &mut Option<(u64, u64)>) -> bool {
    let Some(target) = shared.lock().unwrap().seek_pending else { return true };
    let mut d = demux.lock().unwrap();
    let (demuxer, source) = &mut *d;
    match demuxer.seek_ms(target, &mut **source) {
        Ok(_) => {
            let mut s = shared.lock().unwrap();
            if s.seek_pending == Some(target) {
                s.seek_pending = None;
            }
            true
        }
        Err(Error::NeedData { offset, len }) => {
            *wait(&mut shared.lock().unwrap()) = Some((offset, len));
            false
        }
        Err(e) => {
            shared.lock().unwrap().error = Some(e.to_string());
            false
        }
    }
}

fn video_loop(shared: &Arc<Mutex<Shared>>, notify_fd: c_int, demux: &Demux, codec: mp4::Codec, factory: DecoderFactory) -> Result<(), String> {
    let (kind, config, width, height, hevc) = match &codec {
        mp4::Codec::H264 { avcc, width, height } => (Codec::H264, avcc.clone(), *width, *height, false),
        mp4::Codec::Hevc { hvcc, width, height } => (Codec::Hevc, hvcc.clone(), *width, *height, true),
        mp4::Codec::Aac { .. } => return Err("audio codec in the video track".into()),
    };
    let mut decoder = factory(kind).map_err(|e| e.to_string())?;
    decoder.configure(kind, &config, width, height).map_err(|e| format!("decoder configure: {e}"))?;
    let len_size = mp4::nal_length_size(&config, hevc);
    let parameter_sets = if hevc { mp4::hvcc_parameter_sets(&config) } else { mp4::avcc_parameter_sets(&config) };
    let matrix = convert::matrix_for(height);
    // whether the decoder now hands out pictures in its own buffers
    let mut zero_copy = false;
    let mut generation = 0;
    let mut send_ps = true;
    let mut eof = false;
    let mut flushed = false;
    let mut queue: std::collections::VecDeque<super::Frame> = Default::default();
    // frames before this time are decoded but not shown (seek catch-up)
    let mut drop_until = 0.0f64;
    let mut late_streak = 0;
    // the demuxer is waiting for bytes; poll it slowly
    let mut starved = false;
    loop {
        {
            let mut s = shared.lock().unwrap();
            if s.quit {
                return Ok(());
            }
            if s.generation != generation {
                generation = s.generation;
                drop_until = s.seek_target;
                queue.clear();
                eof = false;
                flushed = false;
                send_ps = true;
                s.video_done = false;
                drop(s);
                decoder.flush().map_err(|e| format!("decoder flush: {e}"))?;
            }
        }
        if !apply_seek(shared, demux, |s| &mut s.video_wait) {
            std::thread::sleep(RETRY);
            continue;
        }
        if queue.len() < LOOKAHEAD && !eof {
            let Some(sample) = fetch(shared, demux, Kind::Video, generation) else { continue };
            starved = false;
            match sample {
                Ok(Some(sample)) => {
                    let had_wait = shared.lock().unwrap().video_wait.take().is_some();
                    if had_wait {
                        notify(notify_fd);
                    }
                    let mut au = Vec::new();
                    if send_ps {
                        au.extend_from_slice(&parameter_sets);
                        send_ps = false;
                    }
                    au.extend(mp4::to_annex_b(&sample.data, len_size));
                    decoder.push(&au, (sample.pts_ms * 1000.0) as i64).map_err(|e| format!("decoder push: {e}"))?;
                }
                Ok(None) => eof = true,
                Err(Error::NeedData { offset, len }) => {
                    starved = true;
                    let mut s = shared.lock().unwrap();
                    if s.video_wait != Some((offset, len)) {
                        s.video_wait = Some((offset, len));
                        drop(s);
                        notify(notify_fd);
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        if eof && !flushed {
            decoder.flush().map_err(|e| format!("decoder flush: {e}"))?;
            flushed = true;
        }
        while let Some(f) = decoder.pull().map_err(|e| format!("decoder pull: {e}"))? {
            queue.push_back(f);
        }
        let Some(front) = queue.front() else {
            if eof {
                let mut s = shared.lock().unwrap();
                if !s.video_done {
                    s.video_done = true;
                    if !s.has_audio {
                        let d = s.duration_ms.unwrap_or_else(|| s.clock.now());
                        s.clock.set(d, false);
                    }
                    notify(notify_fd);
                }
                drop(s);
            }
            // Decode latency is unpredictable (no wake fd from the decoder), so keep the short poll
            // only while playing; paused or finished, nothing arrives that soon.
            let playing = shared.lock().unwrap().playing;
            std::thread::sleep(if starved { RETRY } else { Duration::from_millis(if eof || !playing { 20 } else { 2 }) });
            continue;
        };
        let pts_ms = front.pts_us as f64 / 1000.0;
        if pts_ms < drop_until {
            queue.pop_front();
            continue;
        }
        if queue.len() < LOOKAHEAD && !eof && !starved {
            // keep the decoder fed before blocking on the clock
            let now = shared.lock().unwrap().clock.now();
            if pts_ms - now > 10.0 {
                continue;
            }
        }
        if !wait_until(shared, pts_ms, generation) {
            continue;
        }
        drop_until = 0.0;
        let frame = queue.pop_front().unwrap();
        let (now, viewport, has_audio, want_zero_copy) = {
            let s = shared.lock().unwrap();
            (s.clock.now(), s.viewport, s.has_audio, s.zero_copy)
        };
        if want_zero_copy != zero_copy {
            // frames already queued keep their form; both forms are handled below
            zero_copy = decoder.set_zero_copy(want_zero_copy) && want_zero_copy;
        }
        if now - pts_ms > LATE_MS && late_streak < 8 {
            late_streak += 1;
            continue;
        }
        late_streak = 0;
        let (ow, oh) = convert::fit(frame.width, frame.height, viewport.0, viewport.1);
        let img = match frame.pixels {
            Pixels::Dma(p) if zero_copy => Picture::Gpu(p),
            Pixels::Nv12 { y, uv, stride } => Picture::Rgba(convert::nv12_to_rgba(convert::Nv12 { y: &y, uv: &uv, stride: stride as usize, w: frame.width as usize, h: frame.height as usize }, ow, oh, matrix, CONVERT_THREADS)),
            Pixels::Rgba(img) => Picture::Rgba(convert::rgba_scale(&img, ow, oh)),
            Pixels::Dma(p) => Picture::Rgba(convert::dma_to_rgba(&p, ow, oh, matrix, CONVERT_THREADS)),
        };
        let mut s = shared.lock().unwrap();
        if s.generation == generation {
            if !has_audio && s.playing {
                s.clock.set(pts_ms, true);
            }
            s.frame = Some((img, pts_ms));
            notify(notify_fd);
        }
    }
}

/// The frames the device counts as written after a pause: a device that kept its queue still
/// owes `written`, one that dropped it restarts `played` at 0 and so has nothing owed.
fn written_after_resume(written: u64, played: u64) -> u64 {
    if played == 0 {
        0
    } else {
        written
    }
}

fn audio_loop(shared: Arc<Mutex<Shared>>, notify_fd: c_int, demux: Demux, mut out: Box<dyn AudioOut>) {
    crate::gfx::affinity::audio_thread();
    let asc = match demux.lock().unwrap().0.audio().map(|t| t.codec.clone()) {
        Some(mp4::Codec::Aac { asc, .. }) => asc,
        _ => return,
    };
    let mut decoder = match aac::Decoder::new(&asc) {
        Ok(d) => d,
        Err(e) => {
            shared.lock().unwrap().error = Some(format!("aac: {e}"));
            return;
        }
    };
    let mut resampler = aac::Resampler::new(decoder.rate(), decoder.channels());
    let mut samples = Vec::new();
    let mut pcm = Vec::new();
    let mut generation = 0;
    let mut paused = false;
    let mut eof = false;
    // frames written to the device since the last discard
    let mut written = 0u64;
    loop {
        {
            let mut s = shared.lock().unwrap();
            if s.quit {
                return;
            }
            if s.generation != generation {
                generation = s.generation;
                eof = false;
                written = 0;
                s.audio_done = false;
                drop(s);
                out.discard();
                decoder.reset();
                resampler = aac::Resampler::new(decoder.rate(), decoder.channels());
            }
        }
        if !apply_seek(&shared, &demux, |s| &mut s.audio_wait) {
            std::thread::sleep(RETRY);
            continue;
        }
        let playing = shared.lock().unwrap().playing;
        if playing == paused {
            paused = !playing;
            out.pause(paused);
            // a device that dropped its queue on pause restarts `played` at 0
            written = written_after_resume(written, out.played());
        }
        if paused || eof {
            if eof && out.played() >= written {
                shared.lock().unwrap().audio_done = true;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let Some(sample) = fetch(&shared, &demux, Kind::Audio, generation) else { continue };
        match sample {
            Ok(Some(sample)) => {
                if shared.lock().unwrap().audio_wait.take().is_some() {
                    notify(notify_fd);
                }
                if let Err(e) = decoder.decode(&sample.data, &mut samples) {
                    shared.lock().unwrap().error = Some(format!("aac: {e}"));
                    return;
                }
                resampler.retune(decoder.rate(), decoder.channels());
                pcm.clear();
                resampler.convert(&samples, &mut pcm);
                if let Err(e) = super::audio_out::write_with_retry(&mut *out, &pcm, &super::audio_out::WRITE_RETRY_BACKOFF) {
                    shared.lock().unwrap().error = Some(format!("audio output: {e}"));
                    notify(notify_fd);
                    return;
                }
                written += pcm.len() as u64 / 2;
                // the device is now playing the frame `queued` frames before
                // the end of what we just wrote
                let queued = written.saturating_sub(out.played());
                let end_ms = sample.pts_ms + sample.duration_ms;
                let mut s = shared.lock().unwrap();
                if s.generation == generation && s.playing {
                    s.clock.set(end_ms - queued as f64 * 1000.0 / aac::OUT_RATE as f64, true);
                }
            }
            Ok(None) => eof = true,
            Err(Error::NeedData { offset, len }) => {
                let mut s = shared.lock().unwrap();
                if s.audio_wait != Some((offset, len)) {
                    s.audio_wait = Some((offset, len));
                    drop(s);
                    notify(notify_fd);
                }
                std::thread::sleep(RETRY);
            }
            Err(e) => {
                shared.lock().unwrap().error = Some(e.to_string());
                notify(notify_fd);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::source::MemSource;
    use super::super::Frame;
    use super::*;

    const BASELINE: &[u8] = include_bytes!("testdata/tiny_baseline.mp4");

    /// A decoder that turns every access unit into a flat NV12 picture
    /// whose luma encodes the frame's pts, emitted in push order.
    struct StubDecoder {
        size: (u32, u32),
        pending: std::collections::VecDeque<Frame>,
        pushes: usize,
        configured: Option<(Codec, usize)>,
        fail: bool,
    }

    impl StubDecoder {
        fn new() -> Box<Self> {
            Box::new(StubDecoder { size: (0, 0), pending: Default::default(), pushes: 0, configured: None, fail: false })
        }
    }

    impl VideoDecoder for StubDecoder {
        fn configure(&mut self, codec: Codec, extradata: &[u8], width: u32, height: u32) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("no hardware"));
            }
            self.configured = Some((codec, extradata.len()));
            self.size = (width, height);
            Ok(())
        }
        fn push(&mut self, nal_units: &[u8], pts_us: i64) -> io::Result<()> {
            assert_eq!(&nal_units[..4], &[0, 0, 0, 1], "Annex B");
            if self.pushes == 0 {
                assert_eq!(nal_units[4] & 0x1f, 7, "SPS first");
            }
            self.pushes += 1;
            let (w, h) = (self.size.0 as usize, self.size.1 as usize);
            let luma = (16 + (pts_us / 5_000).min(219)) as u8;
            self.pending.push_back(Frame { width: self.size.0, height: self.size.1, pts_us, pixels: Pixels::Nv12 { y: vec![luma; w * h], uv: vec![128; w * h / 2], stride: w as u32 } });
            Ok(())
        }
        fn pull(&mut self) -> io::Result<Option<Frame>> {
            Ok(self.pending.pop_front())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.pending.clear();
            Ok(())
        }
    }

    /// A memory source the test can grow while the player owns a handle.
    #[derive(Clone)]
    struct Growing(Arc<Mutex<MemSource>>);

    impl Source for Growing {
        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
            self.0.lock().unwrap().read_at(offset, buf)
        }
        fn size(&mut self) -> u64 {
            self.0.lock().unwrap().size()
        }
    }

    /// A player on a small viewport, so debug-build conversions stay fast
    /// enough that no frame is dropped as late while the suite runs.
    fn player(bytes: &[u8]) -> Player {
        let mut p = Player::open_parts(Box::new(MemSource::new(bytes.to_vec())), StubDecoder::new(), Box::new(NullOut::new())).unwrap();
        p.set_viewport(240, 160);
        p
    }

    /// Polls the player's fd like the UI loop, collecting frames until
    /// `done` or the timeout; returns (frames, luma of each).
    fn pump(p: &mut Player, timeout_ms: u64, done: impl Fn(&Player) -> bool) -> Vec<Image> {
        let start = Instant::now();
        let mut frames = Vec::new();
        while start.elapsed().as_millis() < timeout_ms as u128 {
            if crate::bare::sys::wait(&[p.fd()], 20)[0] {
                if let Some(f) = p.take_frame() {
                    frames.push(f);
                }
            }
            if done(p) {
                break;
            }
        }
        frames
    }

    fn luma(img: &Image) -> u32 {
        img.px[img.px.len() / 2] & 255
    }

    #[test]
    fn plays_to_the_end_with_frames_in_order_and_fitted_to_the_viewport() {
        let mut p = player(BASELINE);
        pump(&mut p, 1000, |p| p.duration_ms().is_some());
        assert_eq!(p.duration_ms().map(|d| d.round()), Some(1000.0));
        let frames = pump(&mut p, 3000, |p| p.state() == State::Ended);
        assert_eq!(p.state(), State::Ended);
        assert!(frames.len() >= 20 && frames.len() <= 30, "{} frames", frames.len());
        assert!(frames.iter().all(|f| (f.w, f.h) == (240, 160)), "96x64 fitted into 240x160");
        let lumas: Vec<u32> = frames.iter().map(luma).collect();
        assert!(lumas.windows(2).all(|w| w[1] > w[0]), "monotonic pts: {lumas:?}");
        assert!((p.position_ms() - 1000.0).abs() < 1.0);
    }

    #[test]
    fn default_viewport_is_full_hd() {
        let mut p = Player::open_parts(Box::new(MemSource::new(BASELINE.to_vec())), StubDecoder::new(), Box::new(NullOut::new())).unwrap();
        let frames = pump(&mut p, 2000, |_| false);
        assert!(!frames.is_empty());
        assert!(frames.iter().all(|f| (f.w, f.h) == (1620, 1080)), "96x64 fitted into 1920x1080");
    }

    #[test]
    fn a_dropped_device_queue_resets_the_written_count_on_resume() {
        assert_eq!(written_after_resume(48_000, 0), 0, "dropped: the clock must not jump back");
        assert_eq!(written_after_resume(48_000, 30_000), 48_000, "kept: the queue is still owed");
    }

    #[test]
    fn pause_freezes_and_resume_continues() {
        let mut p = player(BASELINE);
        pump(&mut p, 1000, |p| p.position_ms() > 150.0);
        p.pause();
        assert_eq!(p.state(), State::Paused);
        let at = p.position_ms();
        let frames = pump(&mut p, 200, |_| false);
        assert!(frames.len() <= 1, "no new frames while paused");
        assert!((p.position_ms() - at).abs() < 1.0);
        p.play();
        assert_eq!(p.state(), State::Playing);
        // Frames more than LATE_MS behind the clock are dropped, so under a loaded test run 200 ms of media can pass
        // with few frames shown; wait for 400 ms of media within a longer budget.
        let frames = pump(&mut p, 3000, |p| p.position_ms() > at + 400.0);
        assert!(!frames.is_empty());
        p.stop();
        assert_eq!(p.state(), State::Ended);
    }

    #[test]
    fn seek_restarts_at_the_keyframe() {
        let mut p = player(BASELINE);
        pump(&mut p, 1000, |p| p.position_ms() > 100.0);
        p.seek_ms(800.0);
        let frames = pump(&mut p, 1000, |_| false);
        let frames: Vec<&Image> = frames.iter().filter(|f| luma(f) >= 16 + 500 / 5).collect();
        assert!(!frames.is_empty(), "frames after the seek");
        // keyframe at 500 ms, frames below the target are dropped, so the first shown is >= 800
        assert!(luma(frames[0]) >= 16 + 800 / 5 - 1, "first frame luma {}", luma(frames[0]));
        assert!(p.position_ms() >= 800.0);
    }

    #[test]
    fn missing_data_buffers_then_resumes() {
        let mem = Growing(Arc::new(Mutex::new(MemSource { data: BASELINE.to_vec(), available: 3000, complete: false })));
        let mut p = Player::open_parts(Box::new(mem.clone()), StubDecoder::new(), Box::new(NullOut::new())).unwrap();
        pump(&mut p, 1000, |p| matches!(p.state(), State::Buffering { .. }));
        let State::Buffering { need } = p.state() else { panic!("{:?}", p.state()) };
        assert!(need.0 >= 3000 && need.1 > 0);
        {
            let mut m = mem.0.lock().unwrap();
            m.available = BASELINE.len() as u64;
            m.complete = true;
        }
        pump(&mut p, 3000, |p| p.state() == State::Ended);
        assert_eq!(p.state(), State::Ended);
    }

    #[test]
    fn moov_arriving_late_is_waited_for() {
        let mem = Growing(Arc::new(Mutex::new(MemSource { data: BASELINE.to_vec(), available: 100, complete: false })));
        let mut p = Player::open_parts(Box::new(mem.clone()), StubDecoder::new(), Box::new(NullOut::new())).unwrap();
        pump(&mut p, 1000, |p| matches!(p.state(), State::Buffering { .. }));
        assert!(matches!(p.state(), State::Buffering { .. }));
        assert_eq!(p.duration_ms(), None);
        mem.0.lock().unwrap().available = BASELINE.len() as u64;
        pump(&mut p, 1000, |p| p.duration_ms().is_some());
        assert_eq!(p.duration_ms().map(|d| d.round()), Some(1000.0));
    }

    #[test]
    fn present_to_feeds_a_sink() {
        struct Last(Vec<(usize, f64)>);
        impl super::super::Sink for Last {
            fn present(&mut self, frame: &Image, pts_ms: f64) {
                self.0.push((frame.w, pts_ms));
            }
        }
        let mut p = player(BASELINE);
        let mut sink = Last(Vec::new());
        let start = Instant::now();
        while start.elapsed().as_millis() < 2500 && p.state() != State::Ended {
            if crate::bare::sys::wait(&[p.fd()], 20)[0] {
                p.present_to(&mut sink);
            }
        }
        assert!(sink.0.len() >= 20, "{} frames", sink.0.len());
        assert!(sink.0.iter().all(|f| f.0 == 240));
        assert!(sink.0.windows(2).all(|w| w[1].1 > w[0].1));
        assert!(!p.present_to(&mut sink), "nothing new after the end");
    }

    #[test]
    fn decoder_failure_is_reported() {
        let mut d = StubDecoder::new();
        d.fail = true;
        let p = Player::open_parts(Box::new(MemSource::new(BASELINE.to_vec())), d, Box::new(NullOut::new())).unwrap();
        let start = Instant::now();
        while !matches!(p.state(), State::Error(_)) && start.elapsed().as_secs() < 2 {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(p.state(), State::Error(e) if e.contains("no hardware")), "{:?}", p.state());
    }

    #[test]
    fn open_without_hardware_errors_after_demux() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-media-{}-p", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.mp4");
        std::fs::write(&path, BASELINE).unwrap();
        let p = Player::open(&path).unwrap();
        let start = Instant::now();
        while !matches!(p.state(), State::Error(_)) && start.elapsed().as_secs() < 2 {
            std::thread::sleep(Duration::from_millis(10));
        }
        // the demux succeeds, then the decoder opens or not: no VSP and no libavcodec fails to open it, a desktop with
        // libavcodec decodes (software when VA-API refuses); a demux failure is the one outcome that is wrong
        assert!(matches!(p.state(), State::Error(ref e) if e.contains("VSP") || e.contains("libavcodec") || e.contains("decoder") || e.starts_with("audio output:")) || !matches!(p.state(), State::Error(_)), "{:?}", p.state());
        assert!(Player::open(&dir.join("missing.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
