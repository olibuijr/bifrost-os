//! Recording the composed screen: the parts of a recording that do not talk to the window server.
//!
//! Under the window server the shell's own frame holds only the menu bar and the Dock; the wallpaper and every window are
//! composed there. A recording therefore reads the composed output frame by frame through the window server's
//! `wlr-screencopy` protocol (`screenrec_wl.rs`, which owns the connection and its shm buffer pool) and the frames reach the
//! encoder here.
//!
//! * Pacing: the worker captures on a deadline per frame (`next_tick`); a frame that cannot be taken in time, or finds no free
//!   buffer, or finds the encoder's queue full, is dropped and counted, never waited for. The movie keeps real time: the
//!   encoder places each frame on the timeline (`Timeline`) and repeats the frame that arrives after a gap.
//! * Cropping: the area is chosen in the shell's pixels and the window server's output may have another size (`scale_area`);
//!   the region is computed once from a probe frame, so it holds at any output size and scale.
//! * Pixels: the window server answers with XRGB8888 (B, G, R, X in memory). ffmpeg reads that as `bgr0` with no conversion;
//!   the built-in writer converts to its own pixel order (`xrgb_to_pixels`).
//! * Fallback: when the composed output cannot be read (no screencopy global, the Vulkan renderer that has no readback, a
//!   window server that refuses the frame) the recording does not start (`Unavailable`); the shell's own frame, which would be
//!   a black screen with a menu bar, is never recorded in its place.
//! * Account: the folder and the file belong to the account the shell records for (`owner_for_recording`), not to root.
//!
//! Not yet: dma-buf buffers (the window server offers shm only, so the copy is one read of the shm pool, reused), and damage
//! (the window server answers a copy with damage like a plain copy, so the worker asks for plain copies).

use super::camera::jpeg;
use super::screenrec::{finish_sink, open_avi, spawn_ffmpeg, Area, Encoder, Sink, JPEG_QUALITY, QUEUE};
use crate::gfx::pack;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

/// The fastest a composed recording asks the window server for frames. Each copy makes the window server draw the scene
/// again and read it back, so this is lower than what the display can show.
pub const COMPOSED_MAX_FPS: u32 = 30;
/// Shm buffers the worker rotates through: one being captured, `QUEUE` waiting for the encoder, one being encoded.
pub const POOL: usize = QUEUE + 2;
/// A gap in the capture longer than this (seconds) is not filled with repeated frames; the movie skips ahead.
pub const MAX_GAP_S: u32 = 10;
/// Captures that fail one after another before the recording gives up.
pub const MAX_FAILS: u32 = 8;
/// How long one copy may take before it counts as failed (ms).
pub const GRAB_TIMEOUT_MS: f64 = 2000.0;
/// How often the loop looks at the worker (ms).
pub const STATUS_POLL_MS: f64 = 100.0;
/// ffmpeg's name for XRGB8888 in memory.
pub const PIX_FMT: &str = "bgr0";

/// The frame rate of a composed recording: the display's rate on the GPU path, as every recording, but no more than
/// `COMPOSED_MAX_FPS`.
pub fn composed_fps(display_hz: u32, gpu: bool) -> u32 {
    super::screenrec::frame_rate(display_hz, gpu).min(COMPOSED_MAX_FPS)
}

/// The area `area` (in the shell's `from` pixels; None: all of it) in the window server's `to` pixels, even in both sides
/// for the encoder. Edges are scaled and rounded, so the area covers the same part of the screen at any output size and
/// scale.
pub fn scale_area(area: Option<Area>, from: (u32, u32), to: (u32, u32)) -> Result<Area, String> {
    if from.0 == 0 || from.1 == 0 || to.0 == 0 || to.1 == 0 {
        return Err("the screen has no size".into());
    }
    let a = area.unwrap_or(Area { x: 0, y: 0, w: from.0, h: from.1 });
    if a.w == 0 || a.h == 0 {
        return Err("the area needs a width and height of at least one pixel".into());
    }
    if a.x.checked_add(a.w).is_none_or(|v| v > from.0) || a.y.checked_add(a.h).is_none_or(|v| v > from.1) {
        return Err(format!("the area is outside the {} by {} screen", from.0, from.1));
    }
    let edge = |v: u32, f: u32, t: u32| ((v as u64 * t as u64 + f as u64 / 2) / f as u64) as u32;
    let (x0, x1) = (edge(a.x, from.0, to.0), edge(a.x + a.w, from.0, to.0).min(to.0));
    let (y0, y1) = (edge(a.y, from.1, to.1), edge(a.y + a.h, from.1, to.1).min(to.1));
    super::screenrec::crop_box(Some(Area { x: x0, y: y0, w: x1.saturating_sub(x0), h: y1.saturating_sub(y0) }), to.0, to.1)
}

/// How many frames a movie at `fps` holds when `elapsed_ms` of it have been recorded, counting the one at time zero.
pub fn frames_due(elapsed_ms: f64, fps: u32) -> u64 {
    ((elapsed_ms.max(0.0) * fps.max(1) as f64 / 1000.0) + 1e-6).floor() as u64 + 1
}

/// The next deadline after the capture due at `due` finished at `now` (all in ms), and how many deadlines went by unused.
/// A capture that took longer than a frame interval skips the deadlines it missed instead of racing to catch up.
pub fn next_tick(due: f64, now: f64, interval: f64) -> (f64, u32) {
    let interval = interval.max(0.001);
    let n = ((now - due).max(0.0) / interval).floor() as u64 + 1;
    (due + n as f64 * interval, (n - 1).min(u32::MAX as u64) as u32)
}

/// Where the movie's frames stand: the encoder asks it how many copies of a frame captured at a time to write, so the movie
/// keeps real time whatever was dropped.
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    fps: u32,
    /// frames the movie holds on the timeline (a gap skipped over counts as nothing written, not as held)
    held: u64,
}

impl Timeline {
    pub fn new(fps: u32) -> Self {
        Timeline { fps: fps.max(1), held: 0 }
    }

    /// The copies of a frame captured `elapsed_ms` after the first deadline: more than one fills a gap with the frame that
    /// ended it, none means the frame came before its time and is not needed. A gap longer than `MAX_GAP_S` is skipped over.
    pub fn copies(&mut self, elapsed_ms: f64) -> u32 {
        let due = frames_due(elapsed_ms, self.fps);
        let mut gap = due.saturating_sub(self.held);
        if gap > self.fps as u64 * MAX_GAP_S as u64 {
            self.held = due - 1;
            gap = 1;
        }
        self.held += gap;
        gap as u32
    }
}

/// The pixels of a window server buffer (XRGB8888 or ARGB8888: B, G, R, X in memory, rows `stride` bytes apart) as packed
/// pixels with red in the low byte and opaque alpha, top row first.
pub fn xrgb_to_pixels(bytes: &[u8], w: u32, h: u32, stride: u32, y_invert: bool) -> Vec<u32> {
    let (w, h, stride) = (w as usize, h as usize, stride as usize);
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        let src = if y_invert { h - 1 - row } else { row };
        let line = &bytes[src * stride..src * stride + w * 4];
        out.extend(line.chunks_exact(4).map(|p| pack(p[2], p[1], p[0], 255)));
    }
    out
}

/// Writes the `w` by `h` frame in `bytes` to `out` as tightly packed rows, upside down when `y_invert`.
pub fn write_frame_rows<W: Write>(out: &mut W, bytes: &[u8], w: u32, h: u32, stride: u32, y_invert: bool) -> std::io::Result<()> {
    let (row, stride, h) = (w as usize * 4, stride as usize, h as usize);
    if stride == row && !y_invert {
        return out.write_all(&bytes[..row * h]);
    }
    for i in 0..h {
        let src = if y_invert { h - 1 - i } else { i };
        out.write_all(&bytes[src * stride..src * stride + row])?;
    }
    Ok(())
}

/// What a composed recording counted. The worker and the encoder add to it; the shell reads it for the log and `rec status`.
#[derive(Default)]
pub struct Stats {
    /// frames the worker took from the window server and handed to the encoder
    pub captured: AtomicU64,
    /// frames not taken or not handed on: no free buffer, a full queue, a deadline missed, a frame that came early
    pub dropped: AtomicU64,
    /// frames in the movie
    pub written: AtomicU64,
    /// of those, copies that fill a gap so the movie keeps real time
    pub repeated: AtomicU64,
    /// copies the window server did not answer
    pub failed: AtomicU64,
}

impl Stats {
    pub fn summary(&self) -> String {
        let n = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!("{} captured, {} written ({} repeated to keep time), {} dropped, {} failed", n(&self.captured), n(&self.written), n(&self.repeated), n(&self.dropped), n(&self.failed))
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Why the composed screen cannot be recorded. Each has a message for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// the shell has no way to reach the window server (no session socket)
    NoSession,
    /// the connection failed
    NoConnection(String),
    /// the window server has no screencopy global
    NoScreencopy,
    /// the window server has no output or no shared memory
    NoOutput,
    /// the window server would not give a frame (the Vulkan renderer has no readback)
    Refused,
    /// the window server did not answer in time
    Stalled,
    /// the window server's pixel format is not one the recorder reads
    Format(u32),
    /// the area or the encoder could not be used
    Setup(String),
}

impl Unavailable {
    /// The sentence shown to the person. None of them records anything: no movie is made.
    pub fn message(&self) -> String {
        match self {
            Unavailable::NoSession => "The screen cannot be recorded: this session has no window server connection. Nothing was recorded.".into(),
            Unavailable::NoConnection(e) => format!("The screen cannot be recorded: the window server did not accept the connection ({e}). Nothing was recorded."),
            Unavailable::NoScreencopy => "The screen cannot be recorded: the window server does not offer screen capture. Nothing was recorded.".into(),
            Unavailable::NoOutput => "The screen cannot be recorded: the window server has no screen to read. Nothing was recorded.".into(),
            Unavailable::Refused => "The screen cannot be recorded: the window server refused to give a picture of the screen (the graphics renderer in use cannot be read back). Nothing was recorded.".into(),
            Unavailable::Stalled => "The screen cannot be recorded: the window server did not answer in time. Nothing was recorded.".into(),
            Unavailable::Format(f) => format!("The screen cannot be recorded: the window server's picture format ({f}) is not supported. Nothing was recorded."),
            Unavailable::Setup(e) => format!("The screen cannot be recorded: {e}"),
        }
    }
}

/// A frame of window server memory the encoder can read.
pub trait FrameData: Send {
    fn bytes(&self) -> &[u8];
}

impl FrameData for Vec<u8> {
    fn bytes(&self) -> &[u8] {
        self
    }
}

/// A captured frame on its way to the encoder.
pub struct Captured {
    pub data: Box<dyn FrameData>,
    pub stride: u32,
    pub y_invert: bool,
    /// when it was captured, in ms after the recording's first deadline
    pub at_ms: f64,
}

/// The encoder thread of a composed recording: places each frame on the timeline and writes its copies, until the channel
/// closes. Returns the frames in the movie.
fn encode_composed(rx: Receiver<Captured>, mut sink: Sink, area: Area, fps: u32, stats: &Stats) -> Result<usize, String> {
    let mut timeline = Timeline::new(fps);
    let mut written = 0usize;
    for f in rx {
        let copies = timeline.copies(f.at_ms);
        if copies == 0 {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        match &mut sink {
            Sink::Ffmpeg(child) => {
                let stdin = child.stdin.as_mut().ok_or("ffmpeg has no input")?;
                for _ in 0..copies {
                    write_frame_rows(stdin, f.data.bytes(), area.w, area.h, f.stride, f.y_invert).map_err(|e| format!("ffmpeg: {e}"))?;
                }
            }
            Sink::Avi(writer) => {
                let px = xrgb_to_pixels(f.data.bytes(), area.w, area.h, f.stride, f.y_invert);
                let jpeg = jpeg::encode_pixels(area.w as usize, area.h as usize, &px, JPEG_QUALITY, None);
                for _ in 0..copies {
                    writer.frame(&jpeg).map_err(|e| format!("cannot write a frame: {e}"))?;
                }
            }
        }
        written += copies as usize;
        stats.written.fetch_add(copies as u64, Ordering::Relaxed);
        stats.repeated.fetch_add(copies as u64 - 1, Ordering::Relaxed);
    }
    finish_sink(sink)?;
    Ok(written)
}

/// What a composed recording needs to run on its worker.
pub struct Job {
    pub path: PathBuf,
    pub encoder: Encoder,
    /// the area in the shell's pixels; None: the whole screen
    pub area: Option<Area>,
    /// the shell's screen size, which `area` is in
    pub screen: (u32, u32),
    pub fps: u32,
    pub cursor: bool,
    /// the wait before the first frame (ms)
    pub settle_ms: f64,
    pub owner: Option<(u32, u32)>,
    pub stats: Arc<Stats>,
    /// set by the shell to end the recording
    pub stop: Arc<AtomicBool>,
    /// set by the shell while the screen is off: no frame is taken
    pub paused: Arc<AtomicBool>,
}

/// Opens the encoder for frames of `area` (the window server's pixels) and starts its thread. The channel is the queue to it.
pub fn start_encoder(job: &Job, area: Area) -> Result<(SyncSender<Captured>, JoinHandle<Result<usize, String>>), String> {
    let sink = match &job.encoder {
        Encoder::Ffmpeg(bin) => Sink::Ffmpeg(spawn_ffmpeg(bin, area, job.fps, PIX_FMT, &job.path, job.owner)?),
        Encoder::Avi => Sink::Avi(open_avi(&job.path, area, job.fps, job.owner)?),
    };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Captured>(QUEUE);
    let (fps, stats) = (job.fps, job.stats.clone());
    let worker = std::thread::spawn(move || {
        crate::gfx::affinity::background_thread();
        encode_composed(rx, sink, area, fps, &stats)
    });
    Ok((tx, worker))
}

/// What the worker tells the shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// the probe frame was read and the encoder is open: frames of this size are being recorded
    Running { w: u32, h: u32 },
    /// the composed screen cannot be recorded; no file was made
    Unavailable(Unavailable),
    /// the worker stopped by itself after it had started (the screen changed size, the window server stopped answering, the
    /// encoder failed); the movie so far is kept
    Ended(String),
}

/// Whether the worker has answered the shell yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Running,
}

/// What `Composed::poll` tells the shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// the recording never started: the sentence to show
    Refused(String),
    /// the recording stopped by itself: the sentence to show
    Ended(String),
}

/// The shell's handle on a composed recording's worker.
pub struct Composed {
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    status: Receiver<Status>,
    pub phase: Phase,
}

impl Composed {
    pub fn new(stop: Arc<AtomicBool>, paused: Arc<AtomicBool>, status: Receiver<Status>) -> Self {
        Composed { stop, paused, status, phase: Phase::Starting }
    }

    /// Asks the worker to end the recording.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// While paused (the screen is off) the worker takes no frame.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Reads what the worker has said since the last look. A worker that vanished without saying why is reported too.
    pub fn poll(&mut self) -> Option<Event> {
        loop {
            match self.status.try_recv() {
                Ok(Status::Running { .. }) => self.phase = Phase::Running,
                Ok(Status::Unavailable(u)) => return Some(Event::Refused(u.message())),
                Ok(Status::Ended(why)) => return Some(Event::Ended(why)),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    // the worker has exited without a last word: only a stop the shell asked for is silent
                    if self.stop.load(Ordering::Relaxed) {
                        return None;
                    }
                    return Some(match self.phase {
                        Phase::Starting => Event::Refused(Unavailable::Setup("the capture stopped before it started. Nothing was recorded.".into()).message()),
                        Phase::Running => Event::Ended("The capture stopped unexpectedly.".into()),
                    });
                }
            }
        }
    }
}

/// Starts the worker that captures the composed screen. Without the window server's client libraries (a build without
/// `desktop-wayland`) the worker answers at once that the screen cannot be recorded.
pub fn launch(job: Job) -> (Composed, JoinHandle<Result<usize, String>>) {
    let (tx, rx) = channel();
    let handle = Composed::new(job.stop.clone(), job.paused.clone(), rx);
    #[cfg(feature = "desktop-wayland")]
    let worker = super::screenrec_wl::spawn(job, tx);
    #[cfg(not(feature = "desktop-wayland"))]
    let worker = {
        drop(job);
        std::thread::spawn(move || {
            let u = Unavailable::NoSession;
            let message = u.message();
            let _ = tx.send(Status::Unavailable(u));
            Err(message)
        })
    };
    (handle, worker)
}

/// The account that owns a recording's folder and file: the installed account when the shell runs as root, nobody to hand
/// the file to when the shell runs as the account. A root shell that does not know the account records nothing rather than
/// leave a file the account cannot move or delete.
pub fn owner_for_recording(root: bool, account: Option<(u32, u32)>) -> Result<Option<(u32, u32)>, String> {
    match (root, account) {
        (false, _) => Ok(None),
        (true, Some(a)) => Ok(Some(a)),
        (true, None) => Err("the account the recording belongs to is not known, so nothing was recorded".into()),
    }
}

/// Creates `dir` and the folders above it that are missing, handing each one it made to `owner`.
pub fn create_dir_owned(dir: &Path, owner: Option<(u32, u32)>) -> std::io::Result<()> {
    let mut made = Vec::new();
    let mut at = Some(dir);
    while let Some(d) = at.filter(|d| !d.as_os_str().is_empty()) {
        if d.exists() {
            break;
        }
        made.push(d.to_path_buf());
        at = d.parent();
    }
    std::fs::create_dir_all(dir)?;
    for d in made {
        chown_path(&d, owner)?;
    }
    Ok(())
}

/// Hands `path` to `owner` (uid, gid); nothing when there is none.
pub fn chown_path(path: &Path, owner: Option<(u32, u32)>) -> std::io::Result<()> {
    match owner {
        Some((uid, gid)) => std::os::unix::fs::chown(path, Some(uid), Some(gid)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::photos::library::{civil, tests::scratch};
    use crate::shell::screenrec::{file_name, plan_source, Session, Source, Start};
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn the_frame_rate_follows_the_display_up_to_the_composed_cap() {
        assert_eq!(composed_fps(60, true), COMPOSED_MAX_FPS, "the display is faster than a composed capture can follow");
        assert_eq!(composed_fps(24, true), 24, "a slow display is followed");
        assert_eq!(composed_fps(5, true), crate::shell::screenrec::FPS, "never below the software rate");
        assert_eq!(composed_fps(144, false), crate::shell::screenrec::FPS, "the software path records at its own rate");
    }

    #[test]
    fn the_timeline_keeps_real_time() {
        let mut t = Timeline::new(10);
        assert_eq!(t.copies(0.0), 1, "the first frame");
        assert_eq!(t.copies(100.0), 1, "on time");
        assert_eq!(t.copies(200.0), 1);
        assert_eq!(t.copies(210.0), 0, "a frame that came before its time is not needed");
        assert_eq!(t.copies(500.0), 3, "two frames were dropped, so the frame that ended the gap fills it");
        assert_eq!(t.copies(600.0), 1, "and the next is on time again");
    }

    #[test]
    fn a_gap_longer_than_the_limit_is_skipped_not_filled() {
        let mut t = Timeline::new(10);
        assert_eq!(t.copies(0.0), 1);
        // a stall of 60 s would be 600 frames; the movie skips it
        assert_eq!(t.copies(60_000.0), 1);
        assert_eq!(t.copies(60_100.0), 1, "and carries on in step");
        // a stall just under the limit is filled
        let before = 60_100.0;
        assert_eq!(t.copies(before + 5_000.0), 50);
    }

    #[test]
    fn frames_due_counts_the_first_frame_and_survives_float_rounding() {
        assert_eq!(frames_due(0.0, 30), 1);
        assert_eq!(frames_due(33.0, 30), 1);
        assert_eq!(frames_due(100.0, 30), 4, "3 intervals of 33.3 ms is exactly 100 ms");
        assert_eq!(frames_due(-5.0, 30), 1);
    }

    #[test]
    fn a_late_capture_skips_the_deadlines_it_missed() {
        let interval = 1000.0 / 30.0;
        let (next, skipped) = next_tick(0.0, 5.0, interval);
        assert_eq!(skipped, 0);
        assert!((next - interval).abs() < 1e-9, "the next deadline is one interval on");
        let (next, skipped) = next_tick(0.0, 40.0, interval);
        assert_eq!(skipped, 1, "one deadline went by");
        assert!((next - 2.0 * interval).abs() < 1e-9);
        let (next, skipped) = next_tick(100.0, 90.0, interval);
        assert_eq!((skipped, next > 100.0), (0, true), "a clock that went back does not skip");
    }

    #[test]
    fn the_area_scales_to_the_output_at_any_size_and_scale() {
        let a = Area { x: 100, y: 50, w: 400, h: 300 };
        // same size
        assert_eq!(scale_area(Some(a), (1920, 1080), (1920, 1080)), Ok(a));
        // the output is twice the shell's pixels (a scaled panel)
        assert_eq!(scale_area(Some(a), (960, 540), (1920, 1080)), Ok(Area { x: 200, y: 100, w: 800, h: 600 }));
        // fractional scale: 1.5625
        let r = scale_area(Some(Area { x: 0, y: 0, w: 640, h: 480 }), (1280, 960), (2000, 1500)).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 1000, 750));
        // odd results are made even for the encoder, never grown past the output
        let r = scale_area(Some(Area { x: 1, y: 1, w: 99, h: 99 }), (100, 100), (301, 301)).unwrap();
        assert_eq!((r.w % 2, r.h % 2), (0, 0));
        assert!(r.x + r.w <= 301 && r.y + r.h <= 301);
        // the whole screen of an odd-sized output
        assert_eq!(scale_area(None, (1279, 719), (1279, 719)), Ok(Area { x: 0, y: 0, w: 1278, h: 718 }));
        // a portrait output
        assert_eq!(scale_area(None, (800, 1280), (800, 1280)), Ok(Area { x: 0, y: 0, w: 800, h: 1280 }));
    }

    #[test]
    fn a_bad_area_or_screen_is_refused() {
        assert!(scale_area(Some(Area { x: 90, y: 0, w: 20, h: 10 }), (100, 100), (100, 100)).unwrap_err().contains("outside"));
        assert!(scale_area(Some(Area { x: 0, y: 0, w: 0, h: 10 }), (100, 100), (100, 100)).is_err());
        assert!(scale_area(Some(Area { x: u32::MAX, y: 0, w: 2, h: 2 }), (100, 100), (100, 100)).is_err(), "no overflow");
        assert!(scale_area(None, (0, 0), (100, 100)).is_err());
        assert!(scale_area(None, (100, 100), (0, 0)).is_err());
        assert!(scale_area(Some(Area { x: 0, y: 0, w: 1, h: 1 }), (100, 100), (100, 100)).is_err(), "one pixel rounds to nothing");
    }

    #[test]
    fn window_server_pixels_become_red_green_blue_with_the_alpha_forced_opaque() {
        // B, G, R, X in memory; the second row is the first when y_invert is set
        let bytes = [10, 20, 30, 0, 1, 2, 3, 0];
        assert_eq!(xrgb_to_pixels(&bytes, 1, 2, 4, false), vec![pack(30, 20, 10, 255), pack(3, 2, 1, 255)]);
        assert_eq!(xrgb_to_pixels(&bytes, 1, 2, 4, true), vec![pack(3, 2, 1, 255), pack(30, 20, 10, 255)]);
        // rows with padding after them
        let padded = [10, 20, 30, 0, 99, 99, 1, 2, 3, 0, 99, 99];
        assert_eq!(xrgb_to_pixels(&padded, 1, 2, 6, false), vec![pack(30, 20, 10, 255), pack(3, 2, 1, 255)]);
    }

    #[test]
    fn frame_rows_are_written_tight_and_in_order() {
        let mut out = Vec::new();
        write_frame_rows(&mut out, &[1, 2, 3, 4, 5, 6, 7, 8], 1, 2, 4, false).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8], "no padding: one write");
        let mut out = Vec::new();
        write_frame_rows(&mut out, &[1, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9], 1, 2, 6, false).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8], "the padding after each row is left out");
        let mut out = Vec::new();
        write_frame_rows(&mut out, &[1, 2, 3, 4, 5, 6, 7, 8], 1, 2, 4, true).unwrap();
        assert_eq!(out, vec![5, 6, 7, 8, 1, 2, 3, 4], "upside down when the window server says so");
    }

    #[test]
    fn under_the_window_server_the_shell_frame_is_never_the_fallback() {
        assert_eq!(plan_source(false, false), Ok(Source::ShellFrame), "no window server: the shell's frame is the screen");
        assert_eq!(plan_source(false, true), Ok(Source::ShellFrame));
        assert_eq!(plan_source(true, true), Ok(Source::Composed));
        let refused = plan_source(true, false).unwrap_err();
        assert!(refused.contains("Nothing was recorded"), "the person is told: {refused}");
    }

    #[test]
    fn every_reason_the_screen_cannot_be_read_says_that_nothing_was_recorded() {
        for u in [Unavailable::NoSession, Unavailable::NoConnection("refused".into()), Unavailable::NoScreencopy, Unavailable::NoOutput, Unavailable::Refused, Unavailable::Stalled, Unavailable::Format(7)] {
            assert!(u.message().contains("Nothing was recorded"), "{u:?}");
        }
        assert!(Unavailable::Refused.message().contains("renderer"), "the Vulkan case names the renderer");
    }

    #[test]
    fn a_refusal_before_the_start_is_reported_and_a_silent_stop_is_not() {
        let (tx, rx) = channel();
        let (stop, paused) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let mut c = Composed::new(stop.clone(), paused.clone(), rx);
        assert_eq!(c.poll(), None, "nothing said yet");
        tx.send(Status::Running { w: 10, h: 10 }).unwrap();
        assert_eq!(c.poll(), None);
        assert_eq!(c.phase, Phase::Running);
        tx.send(Status::Ended("The screen size changed.".into())).unwrap();
        assert_eq!(c.poll(), Some(Event::Ended("The screen size changed.".into())));
        // a refusal
        let (tx, rx) = channel();
        let mut c = Composed::new(stop.clone(), paused, rx);
        tx.send(Status::Unavailable(Unavailable::Refused)).unwrap();
        assert!(matches!(c.poll(), Some(Event::Refused(m)) if m.contains("Nothing was recorded")));
        // a worker that vanished before it started is a refusal; after a stop the shell asked for it is silent
        let (tx, rx) = channel::<Status>();
        let mut c = Composed::new(stop.clone(), Arc::new(AtomicBool::new(false)), rx);
        drop(tx);
        assert!(matches!(c.poll(), Some(Event::Refused(_))));
        let (tx, rx) = channel::<Status>();
        let mut c = Composed::new(stop.clone(), Arc::new(AtomicBool::new(false)), rx);
        drop(tx);
        c.request_stop();
        assert_eq!(c.poll(), None);
    }

    #[test]
    fn pausing_is_seen_by_the_worker() {
        let (_tx, rx) = channel::<Status>();
        let (stop, paused) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let c = Composed::new(stop.clone(), paused.clone(), rx);
        c.set_paused(true);
        assert!(paused.load(Ordering::Relaxed));
        c.set_paused(false);
        assert!(!paused.load(Ordering::Relaxed));
        c.request_stop();
        assert!(stop.load(Ordering::Relaxed));
    }

    #[test]
    fn the_encoder_repeats_a_frame_over_a_gap_and_counts_what_it_dropped() {
        let dir = scratch("screenrec-composed-encode");
        let path = dir.join("Screen Recording.avi");
        let area = Area { x: 0, y: 0, w: 4, h: 2 };
        let stats = Arc::new(Stats::default());
        let job = Job {
            path: path.clone(),
            encoder: Encoder::Avi,
            area: None,
            screen: (4, 2),
            fps: 10,
            cursor: false,
            settle_ms: 0.0,
            owner: None,
            stats: stats.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        };
        let (tx, worker) = start_encoder(&job, area).unwrap();
        let frame = |at_ms: f64| Captured { data: Box::new(vec![7u8; 4 * 2 * 4]), stride: 16, y_invert: false, at_ms };
        // at 0 ms, at 100 ms, then a frame at 150 ms that is too early for its slot, then one at 400 ms after a gap
        for at in [0.0, 100.0, 150.0, 400.0] {
            tx.send(frame(at)).unwrap();
        }
        drop(tx);
        assert_eq!(worker.join().unwrap().unwrap(), 5, "frames 0, 1, then 2 to 4 filled by the one that ended the gap");
        assert_eq!(stats.written.load(Ordering::Relaxed), 5);
        assert_eq!(stats.repeated.load(Ordering::Relaxed), 2);
        assert_eq!(stats.dropped(), 1, "the early frame is counted as dropped");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 5, "the movie has the frames the timeline asked for");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ffmpeg_input_is_the_window_servers_pixel_format() {
        let a = crate::shell::screenrec::ffmpeg_args(1920, 1080, 30, PIX_FMT, Path::new("/x/out.mov"));
        let at = |flag: &str| a.iter().position(|s| s == flag).unwrap();
        assert_eq!(a[at("-pix_fmt") + 1], "bgr0", "XRGB8888 in memory is read as it is, with no conversion");
        assert_eq!(a[at("-s") + 1], "1920x1080");
        assert_eq!(a[at("-r") + 1], "30");
    }

    #[test]
    fn a_recording_is_named_by_its_start_time_and_goes_to_the_pictures_folder_of_the_account() {
        let d = civil(0, 0);
        assert_eq!(file_name(&d, "mov"), "Screen Recording 1970-01-01 at 00.00.00.mov");
        // the same folder pictures go to: Desktop of the account's home, never the filesystem root
        let home = crate::shell::dock_stack::pick_home(Some(PathBuf::from("/home/ola")), Some(PathBuf::from("/root")));
        assert_eq!(crate::shell::capture::SaveTo::Desktop.dir(&home).join(file_name(&d, "avi")), PathBuf::from("/home/ola/Desktop/Screen Recording 1970-01-01 at 00.00.00.avi"));
        let none = crate::shell::dock_stack::pick_home(None, None);
        assert_ne!(crate::shell::capture::SaveTo::Desktop.dir(&none), PathBuf::from("/Desktop"));
    }

    #[test]
    fn a_root_shell_records_for_the_account_and_a_user_shell_for_itself() {
        assert_eq!(owner_for_recording(false, Some((1000, 1000))), Ok(None), "the shell already runs as the account");
        assert_eq!(owner_for_recording(false, None), Ok(None));
        assert_eq!(owner_for_recording(true, Some((1000, 1001))), Ok(Some((1000, 1001))));
        assert!(owner_for_recording(true, None).unwrap_err().contains("nothing was recorded"), "never a root-owned file by default");
    }

    #[test]
    fn folders_made_for_a_recording_are_handed_to_the_owner_and_existing_ones_are_left() {
        let dir = scratch("screenrec-owner");
        let me = std::fs::metadata(&dir).unwrap();
        let owner = Some((me.uid(), me.gid()));
        let deep = dir.join("Desktop").join("Recordings");
        create_dir_owned(&deep, owner).unwrap();
        assert!(deep.is_dir());
        let made = std::fs::metadata(&deep).unwrap();
        assert_eq!((made.uid(), made.gid()), (me.uid(), me.gid()));
        // asking again for existing folders is not an error
        create_dir_owned(&deep, owner).unwrap();
        // no owner: nothing is changed
        assert!(chown_path(&deep, None).is_ok());
        let file = deep.join("a.mov");
        std::fs::write(&file, b"x").unwrap();
        chown_path(&file, owner).unwrap();
        let m = std::fs::metadata(&file).unwrap();
        assert_eq!((m.uid(), m.gid()), (me.uid(), me.gid()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_owned_avi_is_never_a_file_that_already_existed() {
        let dir = scratch("screenrec-owner-avi");
        let path = dir.join("a.avi");
        std::fs::write(&path, b"someone else's file").unwrap();
        let me = std::fs::metadata(&dir).unwrap();
        let area = Area { x: 0, y: 0, w: 2, h: 2 };
        let err = open_avi(&path, area, 10, Some((me.uid(), me.gid()))).err().expect("an existing file is not truncated");
        assert!(err.contains("a.avi"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"someone else's file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_composed_recording_refuses_an_area_outside_the_screen_before_anything_starts() {
        let dir = scratch("screenrec-composed-start");
        let path = dir.join("a.avi");
        let start = Start { path: path.clone(), encoder: Encoder::Avi, area: Some(Area { x: 90, y: 0, w: 20, h: 10 }), screen: (100, 100), fps: 60, cursor: false, settle_ms: 0.0, owner: None };
        let err = Session::start_composed(start, 0.0).err().expect("outside the screen");
        assert!(err.contains("outside"), "{err}");
        assert!(!path.exists(), "no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
