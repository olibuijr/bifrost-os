//! Screen recording. The shell records the frames it draws (the same drawing a screenshot takes), cropped to the chosen
//! area, and an encoder writes them. The encoder is the system `ffmpeg` when the PATH holds one (H.264 in a `.mov`), else
//! a built-in Motion JPEG AVI writer. The frame rate follows the display while the GPU path is up (`frame_rate`), else
//! `FPS`. With ffmpeg and the Options menu's Microphone on, the audio service records a WAV take beside the movie, and
//! `mux_audio` adds it to the movie once the service has written it.
//!
//! Recordings start from the screenshot toolbar's Record buttons and from the control port (`rec start`, which
//! `screencapture -v` and `-V` use), and end from the menu bar's Stop Recording item, `rec stop` or the end of `-V`. A
//! finished recording opens in the Video Player through its `open-movie` intent.
//!
//! Not yet: apps cannot record (no capture stream in the SDK), gstreamer is not a second encoder, and the pointer is not
//! drawn into a movie (only into pictures).

use super::camera::jpeg;
use super::photos::library::civil;
use super::services::privacy_permissions::{self, Kind};
use super::Shell;
use crate::gfx::Image;
use bifrost_ui::capture_access::{Media, Status};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Frames per second the shell records at while the GPU path is not up (a frame is drawn in full, in software).
pub const FPS: u32 = 10;
/// The fastest a recording follows the display.
pub const MAX_FPS: u32 = 60;
/// The app whose microphone answer covers the shell's own recordings (the prompt says "Screenshot").
const CAPTURE_APP: &str = "is.olibuijr.screenshot.desktop";
const CAPTURE_NAME: &str = "Screenshot";
/// The Video Player, which opens a finished recording through its `open-movie` intent.
const PLAYER_APP: &str = "is.olibuijr.videoplayer.desktop";
const OPEN_INTENT: &str = "open-movie";
/// How long an open waits for the player's control socket after the player is started.
const OPEN_WAIT: Duration = Duration::from_secs(8);
/// How long a finished recording waits for its audio take before it opens without sound (ms).
const AUDIO_WAIT_MS: f64 = 15_000.0;
/// How soon the loop comes round again while a take is on its way or being muxed (ms).
const MUX_POLL_MS: f64 = 50.0;
/// JPEG quality of the built-in writer.
const JPEG_QUALITY: u8 = 80;
/// Frames queued for the encoder before a frame is skipped instead of waited for.
const QUEUE: usize = 2;

/// Where the built-in AVI writer's `movi` list type fourcc sits: the header below is fixed in size.
const MOVI_AT: u64 = 220;
/// Where the first chunk after the header starts.
const DATA_AT: u64 = 224;

/// A rectangle of the screen, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Which encoder writes the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// The system ffmpeg at this path: H.264 in a `.mov` file.
    Ffmpeg(PathBuf),
    /// The built-in Motion JPEG writer.
    Avi,
}

impl Encoder {
    /// The file extension this encoder writes.
    pub fn extension(&self) -> &'static str {
        match self {
            Encoder::Ffmpeg(_) => "mov",
            Encoder::Avi => "avi",
        }
    }
}

/// The encoder for this machine: ffmpeg when the PATH value holds an executable one, else the built-in writer.
pub fn choose(path_var: Option<&std::ffi::OsStr>) -> Encoder {
    match find_in_path("ffmpeg", path_var) {
        Some(p) => Encoder::Ffmpeg(p),
        None => Encoder::Avi,
    }
}

/// The first executable `name` in the PATH value.
pub fn find_in_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The ffmpeg arguments that read raw RGBA frames of `w` by `h` on stdin and write `out` as H.264 at `fps`.
pub fn ffmpeg_args(w: u32, h: u32, fps: u32, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"].iter().map(|s| s.to_string()).collect();
    a.push(format!("{w}x{h}"));
    a.extend(["-r".to_string(), fps.to_string(), "-i".to_string(), "-".to_string()]);
    a.extend(["-an", "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
    a.push(out.to_string_lossy().into_owned());
    a
}

/// The ffmpeg arguments that copy the video of `movie` and add the WAV take `audio` as AAC, ending with the shorter of the
/// two, into `out`.
pub fn mux_args(movie: &Path, audio: &Path, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-i"].iter().map(|s| s.to_string()).collect();
    a.push(movie.to_string_lossy().into_owned());
    a.push("-i".to_string());
    a.push(audio.to_string_lossy().into_owned());
    a.extend(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-shortest", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
    a.push(out.to_string_lossy().into_owned());
    a
}

/// The frame rate of a recording: the display's refresh rate (within `FPS` to `MAX_FPS`) while the GPU path is up, else
/// `FPS`.
pub fn frame_rate(display_hz: u32, gpu: bool) -> u32 {
    if gpu {
        display_hz.clamp(FPS, MAX_FPS)
    } else {
        FPS
    }
}

/// The area recorded: the whole screen, or `area` inside it, made even in both sides for the encoder.
pub fn crop_box(area: Option<Area>, w: u32, h: u32) -> Result<Area, String> {
    let a = area.unwrap_or(Area { x: 0, y: 0, w, h });
    if a.w == 0 || a.h == 0 {
        return Err("the area needs a width and height of at least one pixel".into());
    }
    if a.x.checked_add(a.w).is_none_or(|v| v > w) || a.y.checked_add(a.h).is_none_or(|v| v > h) {
        return Err(format!("the area is outside the {w} by {h} screen"));
    }
    let even = Area { w: a.w & !1, h: a.h & !1, ..a };
    if even.w == 0 || even.h == 0 {
        return Err("the area is too small to record".into());
    }
    Ok(even)
}

/// The pixels of `area` in `img`, row by row.
pub fn crop_pixels(img: &Image, area: Area) -> Vec<u32> {
    let (x, w) = (area.x as usize, area.w as usize);
    let mut out = Vec::with_capacity(w * area.h as usize);
    for row in area.y as usize..(area.y + area.h) as usize {
        let start = row * img.w + x;
        out.extend_from_slice(&img.px[start..start + w]);
    }
    out
}

/// The RGBA bytes of packed pixels (the low byte is red), as the raw input of the encoder.
fn rgba_bytes(px: &[u32]) -> Vec<u8> {
    px.iter().flat_map(|p| p.to_le_bytes()).collect()
}

fn put(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Writes a Motion JPEG AVI: one `00dc` JPEG chunk per frame and an `idx1` index. The header is written first with the
/// frame count and sizes left at zero; `finish` fills them in.
pub struct AviWriter<W: Write + Seek> {
    out: W,
    /// (offset from the `movi` type fourcc, size) of each frame chunk
    index: Vec<(u32, u32)>,
    /// where the next chunk starts
    pos: u64,
}

impl<W: Write + Seek> AviWriter<W> {
    /// Starts a `w` by `h` video at `fps` frames per second.
    pub fn new(mut out: W, w: u32, h: u32, fps: u32) -> std::io::Result<Self> {
        let mut b: Vec<u8> = Vec::with_capacity(DATA_AT as usize);
        b.extend_from_slice(b"RIFF");
        put(&mut b, 0);
        b.extend_from_slice(b"AVI ");
        // header list: main header, then one video stream
        b.extend_from_slice(b"LIST");
        put(&mut b, 192);
        b.extend_from_slice(b"hdrl");
        b.extend_from_slice(b"avih");
        put(&mut b, 56);
        put(&mut b, 1_000_000 / fps.max(1));
        put(&mut b, 0);
        put(&mut b, 0);
        put(&mut b, 0x10);
        put(&mut b, 0);
        put(&mut b, 0);
        put(&mut b, 1);
        put(&mut b, w * h * 3);
        put(&mut b, w);
        put(&mut b, h);
        for _ in 0..4 {
            put(&mut b, 0);
        }
        b.extend_from_slice(b"LIST");
        put(&mut b, 116);
        b.extend_from_slice(b"strl");
        b.extend_from_slice(b"strh");
        put(&mut b, 56);
        b.extend_from_slice(b"vids");
        b.extend_from_slice(b"MJPG");
        put(&mut b, 0);
        b.extend_from_slice(&[0, 0, 0, 0]);
        put(&mut b, 0);
        put(&mut b, 1);
        put(&mut b, fps);
        put(&mut b, 0);
        put(&mut b, 0);
        put(&mut b, w * h * 3);
        put(&mut b, u32::MAX);
        put(&mut b, 0);
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(b"strf");
        put(&mut b, 40);
        put(&mut b, 40);
        put(&mut b, w);
        put(&mut b, h);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(b"MJPG");
        put(&mut b, w * h * 3);
        for _ in 0..4 {
            put(&mut b, 0);
        }
        // the frames
        b.extend_from_slice(b"LIST");
        put(&mut b, 0);
        b.extend_from_slice(b"movi");
        out.write_all(&b)?;
        Ok(AviWriter { out, index: Vec::new(), pos: DATA_AT })
    }

    /// Appends one JPEG frame.
    pub fn frame(&mut self, jpeg: &[u8]) -> std::io::Result<()> {
        let at = self.pos;
        let len = jpeg.len() as u32;
        self.out.write_all(b"00dc")?;
        self.out.write_all(&len.to_le_bytes())?;
        self.out.write_all(jpeg)?;
        let pad = (len % 2) as u64;
        if pad == 1 {
            self.out.write_all(&[0])?;
        }
        self.pos += 8 + len as u64 + pad;
        self.index.push(((at - MOVI_AT) as u32, len));
        Ok(())
    }

    /// Writes the index, fills in the sizes and frame counts, and returns the writer.
    pub fn finish(mut self) -> std::io::Result<W> {
        let idx_at = self.pos;
        let n = self.index.len() as u32;
        let mut idx = Vec::with_capacity(8 + 16 * self.index.len());
        idx.extend_from_slice(b"idx1");
        put(&mut idx, 16 * n);
        for (offset, size) in &self.index {
            idx.extend_from_slice(b"00dc");
            put(&mut idx, 0x10);
            put(&mut idx, *offset);
            put(&mut idx, *size);
        }
        self.out.write_all(&idx)?;
        let end = idx_at + idx.len() as u64;
        self.patch(4, (end - 8) as u32)?;
        self.patch(216, (idx_at - MOVI_AT) as u32)?;
        self.patch(48, n)?;
        self.patch(140, n)?;
        self.out.seek(SeekFrom::Start(end))?;
        Ok(self.out)
    }

    fn patch(&mut self, at: u64, v: u32) -> std::io::Result<()> {
        self.out.seek(SeekFrom::Start(at))?;
        self.out.write_all(&v.to_le_bytes())
    }
}

/// Where the encoder writes its output.
enum Sink {
    Ffmpeg(Child),
    Avi(AviWriter<File>),
}

fn spawn_ffmpeg(bin: &Path, area: Area, fps: u32, path: &Path) -> Result<Child, String> {
    Command::new(bin)
        .args(ffmpeg_args(area.w, area.h, fps, path))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))
}

fn open_avi(path: &Path, area: Area, fps: u32) -> Result<AviWriter<File>, String> {
    let f = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    AviWriter::new(f, area.w, area.h, fps).map_err(|e| format!("{}: {e}", path.display()))
}

/// The encoder thread: takes frames until the channel closes, then closes the file. Returns the frame count.
fn encode(rx: Receiver<Vec<u32>>, mut sink: Sink, area: Area) -> Result<usize, String> {
    let mut n = 0;
    for px in rx {
        match &mut sink {
            Sink::Ffmpeg(child) => {
                let stdin = child.stdin.as_mut().ok_or("ffmpeg has no input")?;
                stdin.write_all(&rgba_bytes(&px)).map_err(|e| format!("ffmpeg: {e}"))?;
            }
            Sink::Avi(writer) => {
                let jpeg = jpeg::encode_pixels(area.w as usize, area.h as usize, &px, JPEG_QUALITY, None);
                writer.frame(&jpeg).map_err(|e| format!("cannot write a frame: {e}"))?;
            }
        }
        n += 1;
    }
    match sink {
        Sink::Ffmpeg(mut child) => {
            drop(child.stdin.take());
            let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
            if !status.success() {
                return Err(format!("ffmpeg failed ({status})"));
            }
        }
        Sink::Avi(writer) => {
            writer.finish().map_err(|e| format!("cannot finish the file: {e}"))?;
        }
    }
    Ok(n)
}

/// The microphone take a recording makes beside its movie: the WAV the audio service writes, and the ffmpeg that muxes it
/// into the movie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audio {
    pub wav: PathBuf,
    pub ffmpeg: PathBuf,
}

/// A recording in progress: the shell hands it frames, and the encoder thread owns the file.
pub struct Session {
    path: PathBuf,
    area: Area,
    /// frames per second the movie is written at (`frame_rate`)
    pub fps: u32,
    /// the microphone take, when the recording has one
    pub audio: Option<Audio>,
    tx: Option<SyncSender<Vec<u32>>>,
    worker: Option<JoinHandle<Result<usize, String>>>,
    /// the time (ms) the next frame is due
    next_due: f64,
    /// milliseconds between two recorded frames
    interval: f64,
}

/// A finished recording whose audio take is still on its way from the audio service.
pub struct PendingMux {
    movie: PathBuf,
    audio: Audio,
    /// the time (ms) the recording stopped
    since: f64,
}

/// Muxes the take into the movie: the video is copied and the take added as AAC. The movie is replaced only when ffmpeg
/// wrote the new file; a take the audio service never wrote leaves the movie as it is.
fn mux_audio(movie: &Path, audio: &Audio) {
    if audio.wav.is_file() {
        let out = movie.with_extension("muxing.mov");
        let muxed = Command::new(&audio.ffmpeg).args(mux_args(movie, &audio.wav, &out)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit()).status().is_ok_and(|s| s.success()) && std::fs::rename(&out, movie).is_ok();
        if !muxed {
            let _ = std::fs::remove_file(&out);
            eprintln!("springboard: recording: could not add the audio to {}", movie.display());
        }
    }
    let _ = std::fs::remove_file(&audio.wav);
}

impl Session {
    /// Starts recording `area` to `path` with `encoder` at `fps` frames a second; the first frame is due at `t` (ms).
    pub fn start(path: PathBuf, encoder: Encoder, area: Area, fps: u32, t: f64) -> Result<Session, String> {
        let fps = fps.clamp(1, MAX_FPS);
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let sink = match encoder {
            Encoder::Ffmpeg(bin) => Sink::Ffmpeg(spawn_ffmpeg(&bin, area, fps, &path)?),
            Encoder::Avi => Sink::Avi(open_avi(&path, area, fps)?),
        };
        let (tx, rx) = sync_channel::<Vec<u32>>(QUEUE);
        let worker = std::thread::spawn(move || {
            crate::gfx::affinity::background_thread();
            encode(rx, sink, area)
        });
        Ok(Session { path, area, fps, audio: None, tx: Some(tx), worker: Some(worker), next_due: t, interval: 1000.0 / fps as f64 })
    }

    /// Takes the frame `img` when one is due at `t` (ms). A frame the encoder cannot take yet is skipped, not waited for.
    pub fn push(&mut self, img: &Image, t: f64) {
        if t < self.next_due {
            return;
        }
        self.next_due = t + self.interval;
        if let Some(tx) = &self.tx {
            match tx.try_send(crop_pixels(img, self.area)) {
                Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    /// Ends the recording: the encoder drains its queue and closes the file. Returns the file's path.
    pub fn finish(mut self) -> Result<PathBuf, String> {
        drop(self.tx.take());
        let worker = self.worker.take().ok_or("the recording has no encoder")?;
        let frames = worker.join().map_err(|_| "the encoder stopped unexpectedly".to_string())??;
        if frames == 0 {
            return Err(format!("nothing was recorded to {}", self.path.display()));
        }
        Ok(self.path.clone())
    }
}

impl Shell {
    /// Whether a recording is running.
    pub fn recording_active(&self) -> bool {
        self.rec.is_some()
    }

    /// Starts a recording of the whole screen, or of `area` (x, y, width, height in screen pixels), to `file`, or else to
    /// a dated file on the Desktop. Returns the file's path.
    pub fn recording_start(&mut self, area: Option<(u32, u32, u32, u32)>, file: Option<String>, t: f64) -> Result<PathBuf, String> {
        if self.rec.is_some() {
            return Err("already recording".into());
        }
        if !self.screen_on || self.is_locked() {
            return Err("locked".into());
        }
        let area = area.map(|(x, y, w, h)| Area { x, y, w, h });
        let area = crop_box(area, self.w as u32, self.h as u32)?;
        let encoder = choose(std::env::var_os("PATH").as_deref());
        let path = match file.filter(|f| !f.is_empty()) {
            Some(f) => PathBuf::from(f),
            None => {
                let (unix, offset) = self.wall_now(t);
                let d = civil(unix, offset);
                let name = format!("Screen Recording {:04}-{:02}-{:02} at {:02}.{:02}.{:02}.{}", d.year, d.month, d.day, d.hour, d.min, d.sec, encoder.extension());
                self.recording_dir().join(name)
            }
        };
        // the take is muxed with ffmpeg, so only an ffmpeg recording can carry sound
        let ffmpeg = match &encoder {
            Encoder::Ffmpeg(bin) => Some(bin.clone()),
            Encoder::Avi => None,
        };
        let fps = frame_rate(self.rec_display.0, self.rec_display.1);
        let mut session = Session::start(path.clone(), encoder, area, fps, t)?;
        if let Some(bin) = ffmpeg.filter(|_| self.capture_prefs.microphone) {
            session.audio = self.recording_audio(&path, &bin);
        }
        self.rec = Some(session);
        self.rec_clock = t;
        self.dirty = true;
        Ok(path)
    }

    /// Ends the recording and returns its file. The file opens in the Video Player once it is final.
    pub fn recording_stop(&mut self) -> Result<PathBuf, String> {
        let session = self.rec.take().ok_or("not recording")?;
        self.dirty = true;
        let audio = session.audio.clone();
        if audio.is_some() {
            self.audio.record_stop();
        }
        let movie = session.finish()?;
        self.recording_done(movie.clone(), audio);
        Ok(movie)
    }

    /// Records the display's refresh rate and whether the GPU path is up. The loop calls it every pass; the next recording
    /// takes its frame rate from it (`frame_rate`).
    pub fn set_display(&mut self, hz: u32, gpu: bool) {
        self.rec_display = (hz, gpu);
    }

    /// The Options menu turned the microphone on: asks for it now when the person has not answered yet.
    pub fn ask_microphone(&self) {
        if self.capture_prefs.microphone && matches!(privacy_permissions::access(CAPTURE_APP).status(Media::Audio), Status::NotDetermined) {
            privacy_permissions::queue_prompt(CAPTURE_APP, CAPTURE_NAME, Kind::Media(Media::Audio));
        }
    }

    /// The microphone take for a recording to `movie`: starts the audio service recording a WAV beside it. None (the
    /// recording is silent) while the microphone is not allowed or the audio service does not take the start.
    fn recording_audio(&mut self, movie: &Path, ffmpeg: &Path) -> Option<Audio> {
        match privacy_permissions::access(CAPTURE_APP).status(Media::Audio) {
            Status::Authorized => {}
            Status::NotDetermined => {
                privacy_permissions::queue_prompt(CAPTURE_APP, CAPTURE_NAME, Kind::Media(Media::Audio));
                eprintln!("springboard: recording: the microphone needs an answer first; this recording is silent");
                return None;
            }
            _ => {
                eprintln!("springboard: recording: microphone access is off; this recording is silent");
                return None;
            }
        }
        let stem = movie.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let wav = movie.with_file_name(format!(".{stem}.audio.wav"));
        if !self.audio.record_start(&wav.to_string_lossy()) {
            eprintln!("springboard: recording: no audio service; this recording is silent");
            return None;
        }
        Some(Audio { wav, ffmpeg: ffmpeg.to_path_buf() })
    }

    /// A recording is written: it opens in the Video Player, after its audio take is muxed in when it has one.
    fn recording_done(&mut self, movie: PathBuf, audio: Option<Audio>) {
        match audio {
            Some(audio) => self.rec_mux.push(PendingMux { movie, audio, since: self.rec_clock }),
            None => self.open_recording(movie),
        }
    }

    /// Opens a finished recording in the Video Player: the player is started when it is not running, then it is asked
    /// for the movie through its `open-movie` intent.
    fn open_recording(&mut self, movie: PathBuf) {
        let t = self.rec_clock;
        if !self.desktop_open_app(PLAYER_APP, t) {
            eprintln!("springboard: recording: {PLAYER_APP} is not available; {} is saved", movie.display());
            return;
        }
        let path = movie.to_string_lossy().into_owned();
        std::thread::spawn(move || {
            if let Err(e) = super::intents::run_with(PLAYER_APP, OPEN_INTENT, &[("path", path.as_str())], OPEN_WAIT) {
                eprintln!("springboard: recording: {PLAYER_APP} {OPEN_INTENT}: {e}");
            }
        });
    }

    /// Muxes each finished recording whose audio take has arrived (or is overdue) on a thread; the movie opens when the
    /// thread is done (`recording_tick` receives it). Called every pass of the loop.
    fn finish_mux(&mut self, t: f64) {
        if self.rec_mux.is_empty() {
            return;
        }
        let done = self.audio.take_recordings();
        let mut waiting = Vec::new();
        for p in self.rec_mux.drain(..) {
            let arrived = done.iter().any(|(path, _)| Path::new(path) == p.audio.wav.as_path());
            if !arrived && t - p.since < AUDIO_WAIT_MS {
                waiting.push(p);
                continue;
            }
            self.rec_muxing += 1;
            let tx = self.rec_ready.0.clone();
            std::thread::spawn(move || {
                mux_audio(&p.movie, &p.audio);
                let _ = tx.send(p.movie);
            });
        }
        self.rec_mux = waiting;
    }

    /// The menu bar's Stop Recording item: ends the recording, reporting a failure on the log.
    pub fn recording_stop_from_menu(&mut self) {
        if let Err(e) = self.recording_stop() {
            eprintln!("springboard: recording: {e}");
        }
    }

    /// `idle`, or `recording <file>` while a recording runs (the answer to `rec status`).
    pub fn recording_status(&self) -> String {
        match &self.rec {
            Some(r) => format!("recording {}\n", r.path.display()),
            None => "idle\n".into(),
        }
    }

    /// Records the frame that is due at `t` (ms), and opens the recordings whose audio is in. Called once per pass of the
    /// loop.
    pub fn recording_tick(&mut self, t: f64) {
        self.rec_clock = t;
        while let Ok(movie) = self.rec_ready.1.try_recv() {
            self.rec_muxing = self.rec_muxing.saturating_sub(1);
            self.open_recording(movie);
        }
        self.finish_mux(t);
        if !self.rec.as_ref().is_some_and(|r| t >= r.next_due) {
            return;
        }
        let img = self.capture_frame(t);
        if let Some(r) = self.rec.as_mut() {
            r.push(&img, t);
        }
    }

    /// When the loop must come round next for the recording (ms): the next frame while one runs, and soon while a finished
    /// recording waits for its audio take or is being muxed.
    pub fn recording_wake(&self) -> Option<f64> {
        match &self.rec {
            Some(r) => Some(r.next_due),
            None if !self.rec_mux.is_empty() || self.rec_muxing > 0 => Some(self.rec_clock + MUX_POLL_MS),
            None => None,
        }
    }

    /// The folder a recording is saved to: the Options menu's choice in the home directory (the data folder without one).
    fn recording_dir(&self) -> PathBuf {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(self.capture_prefs.save_to.folder()),
            None => self.data_dir.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;
    use crate::shell::photos::library::tests::scratch;
    use std::io::Cursor;

    #[test]
    fn ffmpeg_reads_raw_rgba_and_writes_h264() {
        let a = ffmpeg_args(1280, 720, FPS, Path::new("/x/out.mov"));
        let at = |flag: &str| a.iter().position(|s| s == flag).unwrap();
        assert_eq!(a[at("-s") + 1], "1280x720");
        assert_eq!(a[at("-pix_fmt") + 1], "rgba", "the input is raw RGBA");
        assert_eq!(a[at("-r") + 1], "10");
        assert_eq!(a[at("-i") + 1], "-", "frames come on stdin");
        assert_eq!(a[at("-c:v") + 1], "libx264");
        assert_eq!(a.last().unwrap(), "/x/out.mov");
    }

    #[test]
    fn a_path_entry_must_be_an_executable_file() {
        let dir = scratch("screenrec-path");
        let bin = dir.join("ffmpeg");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        assert_eq!(find_in_path("ffmpeg", Some(dir.as_os_str())), None, "not executable yet");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_in_path("ffmpeg", Some(dir.as_os_str())), Some(bin.clone()));
        assert_eq!(choose(Some(dir.as_os_str())), Encoder::Ffmpeg(bin));
        assert_eq!(choose(None), Encoder::Avi, "no PATH, no ffmpeg");
        assert_eq!(choose(Some(std::ffi::OsStr::new("/nonexistent-bifrost-dir"))), Encoder::Avi);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_area_is_inside_the_screen_and_even() {
        assert_eq!(crop_box(None, 1279, 719), Ok(Area { x: 0, y: 0, w: 1278, h: 718 }));
        assert_eq!(crop_box(Some(Area { x: 10, y: 20, w: 31, h: 40 }), 100, 100), Ok(Area { x: 10, y: 20, w: 30, h: 40 }));
        assert!(crop_box(Some(Area { x: 90, y: 0, w: 20, h: 10 }), 100, 100).unwrap_err().contains("outside"));
        assert!(crop_box(Some(Area { x: 0, y: 0, w: 0, h: 10 }), 100, 100).is_err());
        assert!(crop_box(Some(Area { x: 0, y: 0, w: 1, h: 10 }), 100, 100).is_err(), "one pixel rounds to nothing");
    }

    #[test]
    fn the_pixels_of_the_area_come_row_by_row() {
        let img = Image { w: 3, h: 2, px: vec![1, 2, 3, 4, 5, 6] };
        assert_eq!(crop_pixels(&img, Area { x: 1, y: 0, w: 2, h: 2 }), vec![2, 3, 5, 6]);
    }

    #[test]
    fn the_encoder_gets_red_green_blue_alpha_bytes() {
        assert_eq!(rgba_bytes(&[pack(1, 2, 3, 4)]), vec![1, 2, 3, 4]);
    }

    #[test]
    fn the_avi_has_its_headers_frames_and_index() {
        let mut w = AviWriter::new(Cursor::new(Vec::new()), 16, 8, 10).unwrap();
        w.frame(&[0xff, 0xd8, 1, 2, 3]).unwrap();
        w.frame(&[9, 9]).unwrap();
        let bytes = w.finish().unwrap().into_inner();
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(4) as usize, bytes.len() - 8, "the RIFF size");
        assert_eq!(&bytes[8..12], b"AVI ");
        assert_eq!(&bytes[32..36], b"avih");
        assert_eq!(u32_at(48), 2, "total frames");
        assert_eq!(&bytes[100..104], b"strh");
        assert_eq!(u32_at(140), 2, "stream length");
        assert_eq!(&bytes[MOVI_AT as usize..MOVI_AT as usize + 4], b"movi");
        assert_eq!(u32_at(216) as usize, 248 - MOVI_AT as usize, "the movi list size");
        assert_eq!(&bytes[224..228], b"00dc");
        assert_eq!(u32_at(228), 5);
        assert_eq!(&bytes[238..242], b"00dc", "the odd frame is padded to an even size");
        assert_eq!(&bytes[248..252], b"idx1");
        assert_eq!(u32_at(264), 4, "first frame offset from movi");
        assert_eq!(u32_at(268), 5);
        assert_eq!(u32_at(280), 18, "second frame offset from movi");
        assert_eq!(u32_at(284), 2);
        assert_eq!(bytes.len(), 288);
    }

    #[test]
    fn a_session_writes_the_frames_that_fall_due() {
        let dir = scratch("screenrec-session");
        let path = dir.join("Screen Recording.avi");
        let area = Area { x: 0, y: 0, w: 4, h: 2 };
        let mut s = Session::start(path.clone(), Encoder::Avi, area, FPS, 0.0).unwrap();
        let img = Image { w: 4, h: 2, px: vec![pack(9, 9, 9, 255); 8] };
        s.push(&img, 0.0);
        s.push(&img, 50.0);
        s.push(&img, 100.0);
        assert_eq!(s.finish().unwrap(), path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 2, "two frames were due");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_frame_rate_follows_the_display_only_on_the_gpu_path() {
        assert_eq!(frame_rate(60, true), 60, "the display's rate while the GPU path is up");
        assert_eq!(frame_rate(144, true), MAX_FPS, "capped at the fastest recording");
        assert_eq!(frame_rate(24, true), FPS, "never below the software rate");
        assert_eq!(frame_rate(60, false), FPS, "the software path records at 10 frames a second");
    }

    #[test]
    fn a_fast_session_writes_a_frame_per_display_interval() {
        let dir = scratch("screenrec-fps");
        let path = dir.join("Screen Recording.avi");
        let area = Area { x: 0, y: 0, w: 4, h: 2 };
        let mut s = Session::start(path.clone(), Encoder::Avi, area, 60, 0.0).unwrap();
        assert_eq!(s.fps, 60);
        let img = Image { w: 4, h: 2, px: vec![pack(9, 9, 9, 255); 8] };
        // due at 0, 16.7 and 33.3 ms; the frame at 10 ms is too early
        for t in [0.0, 10.0, 17.0, 34.0] {
            s.push(&img, t);
        }
        s.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 3, "three frames were due");
        assert_eq!(u32::from_le_bytes(bytes[32 + 8..32 + 12].try_into().unwrap()), 1_000_000 / 60, "the header says 60 a second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mux_adds_the_take_as_audio_and_keeps_the_video() {
        let a = mux_args(Path::new("/x/movie.mov"), Path::new("/x/.movie.audio.wav"), Path::new("/x/movie.muxing.mov"));
        let at = |flag: &str| a.iter().position(|s| s == flag).unwrap();
        assert_eq!(a[at("-i") + 1], "/x/movie.mov", "the movie is the first input");
        assert_eq!(a[a.iter().rposition(|s| s == "-i").unwrap() + 1], "/x/.movie.audio.wav", "the take is the second");
        assert_eq!(a[at("-map") + 1], "0:v:0");
        assert_eq!(a[at("-c:v") + 1], "copy", "the video is not encoded again");
        assert_eq!(a[at("-c:a") + 1], "aac");
        assert_eq!(a.last().unwrap(), "/x/movie.muxing.mov");
    }
}
