//! Playing a release: the best compatible file is streamed piece by piece
//! into a fast-start MP4 on disk by the reused engine, and handed to a
//! `Player` once enough of it exists. Decoding is behind the trait.

use super::jobs::Waker;
use deildu_rust::media::Media;
use deildu_rust::stream::{Http, Session};
use deildu_rust::torrent::{Torrent, TorrentFile};
use crate::gfx::Image;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHUNK: usize = 256 * 1024;
/// Bytes on disk before the player is asked to open the file.
pub const OPEN_AFTER: u64 = 8 * 1024 * 1024;
const SAMPLE: Duration = Duration::from_millis(500);

/// What a player is doing, as the playback view needs to know it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PlayState {
    /// nothing open
    #[default]
    Idle,
    Playing,
    Paused,
    /// waiting for data that has not arrived (or been decoded) yet
    Buffering,
    Ended,
    Error(String),
}

/// A media decoder/renderer; the lead plugs the real one in here.
pub trait Player: Send {
    /// Starts playing a (growing) file of the given MIME type.
    fn open(&mut self, path: &Path, mime: &str) -> io::Result<()>;
    /// Stops playback and releases the device.
    fn stop(&mut self);
    /// One line for the progress screen.
    fn status(&self) -> String;
    /// The newest decoded picture, at most once per picture (None = unchanged).
    fn frame(&mut self) -> Option<Image> {
        None
    }
    /// The newest picture like `frame`, but possibly still in the decoder's buffer for the
    /// GPU (only after `set_gpu(true)`).
    fn picture(&mut self) -> Option<crate::media::Picture> {
        self.frame().map(crate::media::Picture::Rgba)
    }
    /// The shell can draw pictures on the GPU: hand them out unconverted from now on.
    fn set_gpu(&mut self, _on: bool) {}
    /// An fd that becomes readable when a picture or status is ready.
    fn fd(&self) -> Option<std::ffi::c_int> {
        None
    }
    /// Resumes playback.
    fn play(&mut self) {}
    /// Pauses playback.
    fn pause(&mut self) {}
    /// Pauses a playing video, resumes a paused one and restarts a finished one.
    fn toggle(&mut self) {
        match self.state() {
            PlayState::Playing | PlayState::Buffering => self.pause(),
            PlayState::Paused => self.play(),
            PlayState::Ended => {
                self.seek_ms(0.0);
                self.play();
            }
            PlayState::Idle | PlayState::Error(_) => {}
        }
    }
    /// Jumps to `ms` from the start (the nearest keyframe at or before it).
    fn seek_ms(&mut self, _ms: f64) {}
    /// The playback position in ms.
    fn position_ms(&self) -> f64 {
        0.0
    }
    /// The length of the video in ms, once known.
    fn duration_ms(&self) -> Option<f64> {
        None
    }
    fn state(&self) -> PlayState {
        PlayState::Idle
    }
    /// The box pictures are scaled to fit (the surface size, either orientation).
    fn set_viewport(&mut self, _w: i32, _h: i32) {}
}

/// Stands in until a decoder exists.
#[derive(Default)]
pub struct StubPlayer {
    asked: Option<PathBuf>,
}

impl Player for StubPlayer {
    fn open(&mut self, path: &Path, _mime: &str) -> io::Result<()> {
        self.asked = Some(path.to_path_buf());
        Err(io::Error::new(io::ErrorKind::Unsupported, "Decoder not available yet"))
    }

    fn stop(&mut self) {
        self.asked = None;
    }

    fn status(&self) -> String {
        "Decoder not available yet".into()
    }
}

/// The file to play, per the compatibility rule (H.264/AAC MP4/M4V; no HEVC).
pub fn pick_file(t: &Torrent) -> Option<TorrentFile> {
    t.preferred_video().cloned()
}

pub fn mime_for(path: &str) -> &'static str {
    if path.to_ascii_lowercase().ends_with(".m4v") {
        "video/x-m4v"
    } else {
        "video/mp4"
    }
}

/// Where a release's file lands: torrents/<id>/<file name>.
pub fn target_path(dir: &Path, id: &str, file: &TorrentFile) -> PathBuf {
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let name = file.path.rsplit('/').next().unwrap_or("video.mp4");
    dir.join("torrents").join(safe).join(name)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub peers: u64,
    pub known_peers: u64,
    pub pieces: u64,
    pub pieces_total: u64,
    pub written: u64,
    pub length: u64,
    /// bytes per second over the last sample
    pub rate: f64,
    pub status: String,
    pub done: bool,
    pub error: Option<String>,
}

impl Progress {
    /// Enough on disk to start a player.
    pub fn ready(&self) -> bool {
        self.error.is_none() && (self.done || self.written >= OPEN_AFTER.min(self.length))
    }

    pub fn fraction(&self) -> f32 {
        if self.length == 0 {
            0.0
        } else {
            (self.written as f64 / self.length as f64) as f32
        }
    }
}

/// Streams one file of a torrent to disk in the background.
pub struct Download {
    session: Arc<Session>,
    progress: Arc<Mutex<Progress>>,
    running: Arc<AtomicBool>,
    pub path: PathBuf,
    pub file: TorrentFile,
}

impl Download {
    pub fn start(torrent: Torrent, file: TorrentFile, http: Arc<dyn Http>, path: PathBuf, waker: Waker) -> Result<Download, String> {
        let pieces_total = torrent.pieces_for(file.offset, file.length).len() as u64;
        let session = Session::new(torrent, file.clone(), http)?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let progress = Arc::new(Mutex::new(Progress { pieces_total, length: file.length, status: "Finding peers…".into(), ..Progress::default() }));
        let running = Arc::new(AtomicBool::new(true));
        session.start();
        let d = Download { session: session.clone(), progress: progress.clone(), running: running.clone(), path, file };
        crate::bare::sys::spawn_named("deildu-write", move || write_loop(session, out, progress, running, waker));
        Ok(d)
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
        self.session.stop();
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A direct MP4 download over HTTP ranges (streaming providers, previews):
/// sequential, resumable by the player once enough is on disk.
pub struct DirectDl {
    media: Arc<deildu_rust::media::Direct>,
    progress: Arc<Mutex<Progress>>,
    running: Arc<AtomicBool>,
    pub path: PathBuf,
    pub name: String,
}

impl DirectDl {
    pub fn start(
        http: Arc<dyn Http>,
        url: String,
        path: PathBuf,
        waker: Waker,
    ) -> Result<DirectDl, String> {
        let media = deildu_rust::media::Direct::new(url, http)?;
        let length = media.length();
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video.mp4".into());
        let progress = Arc::new(Mutex::new(Progress {
            length,
            status: "Streaming from source".into(),
            ..Progress::default()
        }));
        let running = Arc::new(AtomicBool::new(true));
        let d = DirectDl { media: media.clone(), progress: progress.clone(), running: running.clone(), path, name };
        crate::bare::sys::spawn_named("deildu-direct", move || direct_loop(media, out, progress, running, waker));
        Ok(d)
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub fn media(&self) -> Arc<dyn deildu_rust::media::Media> {
        self.media.clone()
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
        self.media.stop();
    }
}

impl Drop for DirectDl {
    fn drop(&mut self) {
        self.stop();
    }
}

fn direct_loop(
    media: Arc<deildu_rust::media::Direct>,
    mut out: std::fs::File,
    progress: Arc<Mutex<Progress>>,
    running: Arc<AtomicBool>,
    waker: Waker,
) {
    let length = media.length();
    let mut pos = 0u64;
    let mut sampled = Instant::now();
    let mut sampled_pos = 0u64;
    let cancel = || !running.load(Ordering::Acquire);
    while pos < length && !cancel() {
        match media.read_virtual(pos, CHUNK, &cancel) {
            Ok(bytes) if !bytes.is_empty() => {
                if let Err(e) = out.write_all(&bytes) {
                    progress.lock().unwrap().error = Some(format!("Write failed: {e}"));
                    break;
                }
                pos += bytes.len() as u64;
            }
            Ok(_) => break,
            Err(e) => {
                if !cancel() {
                    progress.lock().unwrap().error = Some(e);
                }
                break;
            }
        }
        if sampled.elapsed() >= SAMPLE {
            let dt = sampled.elapsed().as_secs_f64();
            let mut p = progress.lock().unwrap();
            p.written = pos;
            p.rate = (pos - sampled_pos) as f64 / dt;
            sampled = Instant::now();
            sampled_pos = pos;
            waker.wake();
        }
    }
    let _ = out.flush();
    {
        let mut p = progress.lock().unwrap();
        p.written = pos;
        p.done = pos >= length;
        if p.done {
            p.status = "Download complete".into();
        }
    }
    waker.wake();
}

/// Either download kind behind the progress screen: a torrent file or a
/// direct provider MP4. Both open in the player once ready.
pub enum AnyDownload {
    Torrent(Download),
    Direct(DirectDl),
}

impl AnyDownload {
    pub fn progress(&self) -> Progress {
        match self {
            AnyDownload::Torrent(d) => d.progress(),
            AnyDownload::Direct(d) => d.progress(),
        }
    }

    pub fn stop(&self) {
        match self {
            AnyDownload::Torrent(d) => d.stop(),
            AnyDownload::Direct(d) => d.stop(),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            AnyDownload::Torrent(d) => &d.path,
            AnyDownload::Direct(d) => &d.path,
        }
    }

    /// File name for the title line and the MIME sniff.
    pub fn label(&self) -> &str {
        match self {
            AnyDownload::Torrent(d) => d.file.path.rsplit('/').next().unwrap_or("video.mp4"),
            AnyDownload::Direct(d) => &d.name,
        }
    }

    /// The byte source for the TV server: torrent session or origin stream.
    pub fn media(&self) -> Arc<dyn deildu_rust::media::Media> {
        match self {
            AnyDownload::Torrent(d) => d.session.clone(),
            AnyDownload::Direct(d) => d.media(),
        }
    }
}

fn write_loop(session: Arc<Session>, mut out: std::fs::File, progress: Arc<Mutex<Progress>>, running: Arc<AtomicBool>, waker: Waker) {
    let length = session.file.length;
    let mut pos = 0u64;
    let mut sampled = Instant::now();
    let mut sampled_pos = 0u64;
    let cancel = || !running.load(Ordering::Acquire);
    let sample = |pos: u64, rate: f64, progress: &Mutex<Progress>| {
        let stats = session.stats();
        let mut p = progress.lock().unwrap();
        p.peers = stats["peers"].as_u64().unwrap_or(0);
        p.known_peers = stats["knownPeers"].as_u64().unwrap_or(0);
        p.pieces = stats["pieces"].as_u64().unwrap_or(0);
        p.written = pos;
        p.rate = rate;
        p.status = stats["status"].as_str().unwrap_or("").to_string();
    };
    while pos < length && !cancel() {
        match session.read_virtual(pos, CHUNK, &cancel) {
            Ok(bytes) if !bytes.is_empty() => {
                if let Err(e) = out.write_all(&bytes) {
                    progress.lock().unwrap().error = Some(format!("Write failed: {e}"));
                    break;
                }
                pos += bytes.len() as u64;
            }
            Ok(_) => break,
            Err(e) => {
                if !cancel() {
                    progress.lock().unwrap().error = Some(e);
                }
                break;
            }
        }
        if sampled.elapsed() >= SAMPLE {
            let dt = sampled.elapsed().as_secs_f64();
            sample(pos, (pos - sampled_pos) as f64 / dt, &progress);
            sampled = Instant::now();
            sampled_pos = pos;
            waker.wake();
        }
    }
    let _ = out.flush();
    {
        sample(pos, 0.0, &progress);
        let mut p = progress.lock().unwrap();
        p.done = pos >= length;
        if p.done {
            p.status = "Download complete".into();
        }
    }
    waker.wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deildu::jobs::Pool;
    use deildu_rust::torrent;

    fn file(path: &str, len: u64, off: u64) -> TorrentFile {
        TorrentFile { path: path.into(), length: len, offset: off }
    }

    fn torrent(files: Vec<TorrentFile>) -> Torrent {
        let total = files.iter().map(|f| f.length).sum();
        Torrent { name: "rel".into(), info_hash: [1; 20], piece_length: 32, hashes: vec![[0; 20]; (total as usize).div_ceil(32)], total_length: total, files, trackers: vec![], web_seeds: vec![] }
    }

    #[test]
    fn picks_the_best_compatible_file() {
        let t = torrent(vec![file("a/movie.x265.mp4", 100, 0), file("a/movie.x264.aac.mp4", 90, 100), file("a/sample.mkv", 10, 190), file("a/big.mp4", 300, 200)]);
        assert_eq!(pick_file(&t).unwrap().path, "a/movie.x264.aac.mp4", "codec tags beat size");
        let none = torrent(vec![file("x.mkv", 10, 0), file("y.avi", 10, 10), file("z.hevc.mp4", 10, 20)]);
        assert!(pick_file(&none).is_none());
        assert_eq!(mime_for("a/b.M4V"), "video/x-m4v");
        assert_eq!(mime_for("a/b.mp4"), "video/mp4");
        let p = target_path(Path::new("/d"), "12/3", &file("a/Movie Name.mp4", 1, 0));
        assert_eq!(p, PathBuf::from("/d/torrents/123/Movie Name.mp4"));
    }

    #[test]
    fn stub_player_reports_no_decoder() {
        let mut p = StubPlayer::default();
        let e = p.open(Path::new("/x.mp4"), "video/mp4").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        assert_eq!(p.status(), "Decoder not available yet");
        p.stop();
    }

    #[test]
    fn progress_ready_and_fraction() {
        let mut p = Progress { length: 100, written: 50, ..Progress::default() };
        assert!(!p.ready(), "small files wait for completion");
        assert_eq!(p.fraction(), 0.5);
        p.written = 100;
        assert!(p.ready());
        p.length = 100 * 1024 * 1024;
        assert!(!p.ready());
        p.written = OPEN_AFTER;
        assert!(p.ready());
        p.error = Some("x".into());
        assert!(!p.ready());
        assert_eq!(Progress::default().fraction(), 0.0);
    }

    /// A web seed serving `data` by byte range.
    struct Seed(Vec<u8>);
    impl Http for Seed {
        fn get(&self, _: &str, range: Option<(u64, u64)>, _: usize) -> Result<deildu_rust::stream::Response, String> {
            let (a, b) = range.ok_or("no range")?;
            let body = self.0.get(a as usize..=b as usize).ok_or("bad range")?.to_vec();
            Ok(deildu_rust::stream::Response { status: 206, body, final_url: String::new(), content_range: format!("bytes {a}-{b}/{}", self.0.len()) })
        }
    }

    #[test]
    fn download_writes_the_virtual_stream_to_disk() {
        // one 32-byte piece: ftyp + moov + mdat, fetched from a web seed
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&[0, 0, 0, 8, b'f', b't', b'y', b'p']);
        bytes[8..16].copy_from_slice(&[0, 0, 0, 8, b'm', b'o', b'o', b'v']);
        bytes[16..24].copy_from_slice(&[0, 0, 0, 16, b'm', b'd', b'a', b't']);
        let f = file("video.mp4", 32, 0);
        let mut t = torrent(vec![f.clone()]);
        t.hashes = vec![torrent::sha1(&bytes)];
        t.web_seeds = vec!["http://seed.test/video.mp4".into()];
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-deildu-play"));
        let _ = std::fs::remove_dir_all(&dir);
        let mut pool: Pool<()> = Pool::new(1).unwrap();
        let path = target_path(&dir, "1", &f);
        let d = Download::start(t, f, Arc::new(Seed(bytes.clone())), path.clone(), pool.waker()).unwrap();
        assert_eq!(d.progress().pieces_total, 1);
        let start = Instant::now();
        while !d.progress().done && start.elapsed() < Duration::from_secs(5) {
            crate::bare::sys::wait(&[pool.fd()], 100);
            pool.drain();
        }
        let p = d.progress();
        assert!(p.done, "{p:?}");
        assert_eq!((p.written, p.pieces), (32, 1));
        assert!(p.ready());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        d.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
