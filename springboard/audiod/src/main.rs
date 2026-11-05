//! The media player service (`is.olibuijr.mediaplayer` on the audio server): decodes and plays music and records voice memos as a client of bifrost-audiod. Music, Podcasts,
//! Voice Memos and the shell talk to it over `BIFROST_SERVICE_SOCK` (frames from
//! `bifrost_proto::svc::audio`). Playback belongs to the app whose connection set the queue: it stops when that app
//! quits or its connection drops, while the shell's controls (media keys, Now Playing) only steer it.

use bifrost_proto::svc::audio::{Cmd, Evt, Live, Repeat as WireRepeat, Snapshot, Track as WireTrack};
use bifrost_proto::svc;
use springboard::media::audioplayer::{AudioPlayer, Snapshot as PlayerSnapshot, Track};
use springboard::media::capture::{wav_bytes, Recorder, Take};
use springboard::media::queue::Repeat;
use springboard::media::stream::Phase;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The wire id and socket name stay `is.olibuijr.audio` so the apps (proto::svc::audio) do not change.
const ID: &str = "is.olibuijr.audio";
/// What this service is on the audio server: a plain client named `is.olibuijr.mediaplayer`
/// (`media::speaker::CLIENT`), never a device owner. It opens no `/dev/snd` node.
const CLIENT: &str = springboard::media::speaker::CLIENT;
const VERSION: &str = env!("CARGO_PKG_VERSION");

// same helpers as audiod/netd; keep them in step
/// Locks the core even after a client handler panicked while holding it. Each command applies whole, so the
/// state stays usable, and one bad connection must not silence the sound for every other client.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `Condvar::wait` with the same poison tolerance as [`lock`].
fn wait<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    cv.wait(g).unwrap_or_else(PoisonError::into_inner)
}

/// A subscriber: the write side of its connection (shared with the connection's own replies, so frames never
/// interleave) and the broadcast sequence number it joined at.
struct Client {
    id: u32,
    out: Arc<Mutex<UnixStream>>,
    since: u64,
}

/// A recording in progress and the connection that started it.
struct Active {
    recorder: Recorder,
    path: PathBuf,
    owner: u32,
}

/// A finished take whose file could not be written: it is kept so the owner can stop again to retry.
struct Kept {
    take: Take,
    path: PathBuf,
    owner: u32,
}

/// Longest take kept in memory (16-bit mono: about 345 MB an hour at 48 kHz). The service ends and saves a take that
/// reaches it.
const MAX_RECORD_MS: u64 = 2 * 60 * 60 * 1000;
/// Broadcast frames queued for the sender thread; the oldest go first (a state frame is superseded by the next).
const MAX_OUTBOX: usize = 64;

struct Core {
    player: AudioPlayer,
    recorder: Option<Active>,
    /// the connection whose `Recorder::start` is in flight (the core lock is not held meanwhile)
    starting: Option<u32>,
    kept: Option<Kept>,
    muted: bool,
    /// The connection whose `PlayQueue` set the queue now playing. Playback belongs to that app: when its connection
    /// ends (it quit, crashed or was killed) the queue is cleared, so nothing it started, a radio stream above all,
    /// keeps playing or streaming without it. Play, pause and skip from others (the shell's media keys) do not move it.
    playback_owner: Option<u32>,
    clients: Vec<Client>,
    /// The output route the shell reported last (headphones, reason), told to every subscriber and to new ones.
    route: Option<(bool, u8)>,
    /// Frames to broadcast, numbered; the sender thread writes them outside the core lock.
    outbox: Vec<(u64, Vec<u8>)>,
    seq: u64,
    /// something changed that subscribers may need to hear about (a command ran, the engine signalled)
    dirty: bool,
}

fn track_in(t: &WireTrack) -> Track {
    match &t.url {
        Some(url) => Track::live(t.id, url.clone()),
        None => Track::file(t.id, PathBuf::from(&t.path)),
    }
}

fn track_out(t: &Track) -> WireTrack {
    WireTrack { id: t.id, path: t.path.to_string_lossy().into_owned(), url: t.url.clone() }
}

fn snapshot(s: PlayerSnapshot, muted: bool) -> Snapshot {
    Snapshot {
        track: s.track.as_ref().map(track_out),
        index: s.index.map(|i| i as u32),
        playing: s.playing,
        position_ms: s.position_ms,
        duration_ms: s.duration_ms,
        shuffle: s.shuffle,
        repeat: match s.repeat {
            Repeat::Off => WireRepeat::Off,
            Repeat::All => WireRepeat::All,
            Repeat::One => WireRepeat::One,
        },
        queue_len: s.queue_len as u32,
        volume: s.volume,
        muted,
        rate: s.rate,
        error: s.error,
        live: s.live.map(|l| Live {
            phase: match l.phase {
                Phase::Buffering => 0,
                Phase::Live => 1,
                Phase::Reconnecting(n) => n + 2,
            },
            title: l.title,
            error: l.error,
        }),
    }
}

/// Where recordings may be written: the apps' data directories under the service root (the shell directory the
/// service was started with).
fn record_root() -> PathBuf {
    let root = std::env::var_os("BIFROST_SHELL_DIR")
        .or_else(|| std::env::var_os("BIFROST_SERVICE_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/springboard"));
    root.join("appdata")
}

/// A recording path the service accepts: absolute, no `..`, a `.wav` name, and a directory that exists inside
/// `root` once symlinks are resolved. Returns the path with its directory resolved. The file itself is only ever
/// created new (see `write_wav_atomic`), so an existing file or link at the name is never followed.
fn validate_record_path(path: &str, root: &Path) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err("the recording path must be absolute".into());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("the recording path must not contain ..".into());
    }
    if p.extension().and_then(|e| e.to_str()) != Some("wav") {
        return Err("the recording path must end in .wav".into());
    }
    let name = p.file_name().ok_or_else(|| "the recording path has no file name".to_string())?;
    let dir = p.parent().ok_or_else(|| "the recording path has no directory".to_string())?;
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("the recording directory is not usable: {e}"))?;
    let root = std::fs::canonicalize(root).map_err(|e| format!("the recordings area is not available: {e}"))?;
    if !dir.starts_with(&root) {
        return Err("recordings can only be saved inside an app's data directory".into());
    }
    Ok(dir.join(name))
}

/// The saved privacy answers the shell writes beside the app data (`privacy_permissions::persist_grants`).
const GRANTS_FILE: &str = "privacy-grants.tsv";
const GRANTS_HEADER: &str = "springboard-privacy-grants-v1";

/// The app a validated recording path belongs to: the first directory under the (canonical) `root`, which is the
/// app's data directory `appdata/<id>`. None for a path directly in `root`.
fn record_app(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = rel.components();
    let first = parts.next()?;
    // the file itself is the last component; a path straight in `root` names no app
    parts.next()?;
    match first {
        Component::Normal(id) => id.to_str().map(str::to_string),
        _ => None,
    }
}

/// Whether the saved grants say the person allowed `app` the microphone. Anything else (no file, another header, not
/// answered, denied) refuses: an unanswered question never grants.
fn microphone_granted(grants: &str, app: &str) -> bool {
    let mut lines = grants.lines();
    lines.next() == Some(GRANTS_HEADER) && lines.any(|l| l.split('\t').eq(["media", app, "microphone", "1"]))
}

/// Refuses a take for an app the person has not allowed the microphone (Settings > Privacy > App Permissions, or the
/// shell's Allow sheet). Every app's recording comes through this service, so this is where consent is enforced.
fn check_microphone(path: &Path, appdata: &Path) -> Result<(), String> {
    let root = std::fs::canonicalize(appdata).map_err(|e| format!("the recordings area is not available: {e}"))?;
    let app = record_app(path, &root).ok_or_else(|| "recordings can only be saved inside an app's data directory".to_string())?;
    let grants = appdata.parent().map(|r| std::fs::read_to_string(r.join(GRANTS_FILE)).unwrap_or_default()).unwrap_or_default();
    if microphone_granted(&grants, &app) {
        Ok(())
    } else {
        Err(format!("microphone access is not allowed for {app}"))
    }
}

/// Writes `bytes` to `path` without ever following a link or replacing a file in place: a new temporary file next to
/// it (created exclusively, so a link at that name is refused), synced, then renamed over the target. A crash leaves
/// the old file or none, never a truncated one.
fn write_wav_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the target is a symbolic link"));
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Whole milliseconds a take covers.
fn take_ms(take: &Take) -> u64 {
    take.samples.len() as u64 / take.format.channels.max(1) as u64 * 1000 / take.format.sample_rate.max(1.0) as u64
}

/// Broadcast frames a client that joined at `since` still has to receive.
fn due(frames: &[(u64, Vec<u8>)], since: u64) -> impl Iterator<Item = &Vec<u8>> {
    frames.iter().filter(move |(n, _)| *n > since).map(|(_, b)| b)
}

impl Core {
    fn new(player: AudioPlayer) -> Core {
        Core {
            player,
            recorder: None,
            starting: None,
            kept: None,
            muted: true,
            playback_owner: None,
            clients: Vec::new(),
            route: None,
            outbox: Vec::new(),
            seq: 0,
            dirty: false,
        }
    }

    /// Connection `conn` ended: it is no longer a subscriber, and playback it owns ends with it (the queue cleared, a
    /// live station's connection closed). The remaining subscribers hear the stopped state.
    fn client_gone(&mut self, conn: u32) {
        self.clients.retain(|c| c.id != conn);
        if self.playback_owner != Some(conn) {
            return;
        }
        self.playback_owner = None;
        self.player.clear();
        eprintln!("audio.player: connection {conn} owned playback and closed; playback stopped");
        let evt = self.state();
        self.send_all(&evt);
        self.dirty = true;
        WAKE.notify_all();
    }

    fn state(&self) -> Evt {
        Evt::State(snapshot(self.player.snapshot(), self.muted))
    }

    fn record_state(&self) -> Evt {
        match &self.recorder {
            Some(a) => Evt::Record { active: true, elapsed_ms: a.recorder.elapsed_ms(), level: a.recorder.level(), error: a.recorder.error() },
            None => Evt::Record { active: false, elapsed_ms: 0, level: 0.0, error: None },
        }
    }

    /// Queues a frame for every subscriber. Nothing is written here: the sender thread does that outside the lock,
    /// so a slow subscriber never holds up a command.
    fn send_all(&mut self, evt: &Evt) {
        if self.clients.is_empty() {
            return;
        }
        self.seq += 1;
        self.outbox.push((self.seq, evt.encode()));
        if self.outbox.len() > MAX_OUTBOX {
            self.outbox.remove(0);
        }
        OUTBOX.notify_all();
    }

    /// Applies one command; the reply for the sender, if any.
    fn apply(&mut self, cmd: Cmd) -> Option<Evt> {
        match cmd {
            Cmd::Ping => return Some(Evt::Pong { id: ID.into(), version: VERSION.into() }),
            Cmd::Subscribe => return Some(self.state()),
            Cmd::PlayQueue { tracks, start, shuffle, seed } => {
                let tracks: Vec<Track> = tracks.iter().map(track_in).collect();
                self.player.play_queue(tracks, start as usize, shuffle, seed);
            }
            Cmd::Play => self.player.play(),
            Cmd::Pause => self.player.pause(),
            Cmd::Toggle => self.player.toggle(),
            Cmd::Next => self.player.next(),
            Cmd::Prev => self.player.prev(),
            Cmd::JumpTo(i) => self.player.jump_to(i as usize),
            Cmd::SeekMs(ms) => self.player.seek_ms(ms),
            Cmd::SetShuffle { on, seed } => self.player.set_shuffle(on, seed),
            Cmd::SetRepeat(r) => self.player.set_repeat(match r {
                WireRepeat::Off => Repeat::Off,
                WireRepeat::All => Repeat::All,
                WireRepeat::One => Repeat::One,
            }),
            Cmd::SetVolume(v) => self.player.set_volume(v),
            Cmd::SetPan(p) => self.player.set_pan(p),
            Cmd::FadeVolume { target, ms } => self.player.fade_volume(target, ms),
            Cmd::SetRoute { headphones, reason } => {
                self.route = Some((headphones, reason));
                self.send_all(&Evt::Route { headphones, reason });
                return None;
            }
            Cmd::SetRate(r) => self.player.set_rate(r),
            Cmd::SetCrossfade(ms) => self.player.set_crossfade_ms(ms),
            Cmd::SetMuted(m) => {
                self.muted = m;
                self.player.set_muted(m);
            }
            Cmd::Stop => self.player.pause(),
            // the recording commands talk to the audio server and the disk: `serve` runs them without the lock
            Cmd::RecordStart { .. } | Cmd::RecordStop => return None,
        }
        let evt = self.state();
        self.send_all(&evt);
        self.dirty = true;
        WAKE.notify_all();
        None
    }
}

/// Writes one event to a connection (whole frame, under the connection's write lock).
fn send_to(out: &Mutex<UnixStream>, evt: &Evt) -> bool {
    svc::write_frame(&mut *lock(out), &evt.encode()).is_ok()
}

fn record_error(message: String) -> Evt {
    Evt::Record { active: false, elapsed_ms: 0, level: 0.0, error: Some(message) }
}

/// `RecordStart` from connection `conn`. The audio server round trips (connect, default device, start) run with the
/// core lock released, so a slow server cannot freeze the other clients. True when this connection now owns a take.
fn start_recording(core: &Mutex<Core>, conn: u32, path: &str, out: &Mutex<UnixStream>) -> bool {
    let appdata = record_root();
    let path = match validate_record_path(path, &appdata).and_then(|p| check_microphone(&p, &appdata).map(|_| p)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("audio.player: connection {conn}: record refused: {e}");
            send_to(out, &record_error(e));
            return false;
        }
    };
    let leftover = {
        let mut c = lock(core);
        if c.recorder.is_some() || c.starting.is_some() {
            // a take is running (or about to): report it, change nothing
            let evt = c.record_state();
            c.send_all(&evt);
            return false;
        }
        c.starting = Some(conn);
        c.kept.take()
    };
    if let Some(k) = leftover {
        // a take that could not be written and was never retried: one more try before it is replaced
        if let Err(e) = write_wav_atomic(&k.path, &wav_bytes(&k.take.format, &k.take.samples)) {
            eprintln!("audio.player: unsaved take for {} dropped: {e}", k.path.display());
        }
    }
    let started = Recorder::start();
    let mut c = lock(core);
    c.starting = None;
    match started {
        Ok(recorder) => {
            c.recorder = Some(Active { recorder, path, owner: conn });
            let evt = c.record_state();
            c.send_all(&evt);
            c.dirty = true;
            WAKE.notify_all();
            true
        }
        Err(e) => {
            drop(c);
            send_to(out, &record_error(e.to_string()));
            false
        }
    }
}

/// Ends the take `conn` owns (any take with `None`: the length cap) and saves it, with the lock released while the
/// capture thread is joined and the file is written. A take that cannot be written is kept for the owner to retry.
fn stop_recording(core: &Mutex<Core>, conn: Option<u32>, report: bool) {
    enum Job {
        Stop(Active),
        Retry(Kept),
        None,
    }
    let job = {
        let mut c = lock(core);
        let owns = |owner: u32| conn.is_none_or(|id| id == owner);
        if c.recorder.as_ref().is_some_and(|a| owns(a.owner)) {
            Job::Stop(c.recorder.take().expect("checked above"))
        } else if c.recorder.is_none() && c.kept.as_ref().is_some_and(|k| owns(k.owner)) {
            Job::Retry(c.kept.take().expect("checked above"))
        } else {
            Job::None
        }
    };
    let (take, path, owner) = match job {
        Job::Stop(a) => (a.recorder.stop(), a.path, a.owner),
        Job::Retry(k) => (k.take, k.path, k.owner),
        Job::None => {
            // not this connection's take (or none): say how things stand and leave them
            if report {
                let mut c = lock(core);
                let evt = c.record_state();
                c.send_all(&evt);
            }
            return;
        }
    };
    let ms = take_ms(&take);
    let written = write_wav_atomic(&path, &wav_bytes(&take.format, &take.samples));
    let mut c = lock(core);
    let evt = match written {
        Ok(()) => Evt::RecordDone { path: path.to_string_lossy().into_owned(), duration_ms: ms },
        Err(e) => {
            let message = format!("{e}; the recording is kept, stop again to retry");
            c.kept = Some(Kept { take, path, owner });
            Evt::Record { active: false, elapsed_ms: ms, level: 0.0, error: Some(message) }
        }
    };
    c.send_all(&evt);
    let evt = c.record_state();
    c.send_all(&evt);
    c.dirty = true;
    WAKE.notify_all();
}

fn serve(core: Arc<Mutex<Core>>, stream: UnixStream, conn: u32) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut stream = stream;
    // everything this connection sends goes through one write lock, shared with the broadcast sender
    let out = Arc::new(Mutex::new(writer));
    let mut subscribed = false;
    while let Ok(frame) = svc::read_frame(&mut stream) {
        let Ok(cmd) = Cmd::decode(&frame) else { continue };
        match cmd {
            Cmd::RecordStart { path } => {
                start_recording(&core, conn, &path, &out);
                continue;
            }
            Cmd::RecordStop => {
                stop_recording(&core, Some(conn), true);
                continue;
            }
            _ => {}
        }
        let subscribe = cmd == Cmd::Subscribe;
        let starts_queue = matches!(cmd, Cmd::PlayQueue { .. });
        let mut c = lock(&core);
        let reply = c.apply(cmd);
        if starts_queue {
            // the app that set the queue owns the playback until another sets one or it goes away
            c.playback_owner = Some(conn);
        }
        let first = subscribe && !subscribed;
        if first {
            subscribed = true;
            let since = c.seq;
            c.clients.push(Client { id: conn, out: out.clone(), since });
            WAKE.notify_all();
        }
        let route = if subscribe { c.route } else { None };
        // A first subscribe takes its write lock before the core is let go, so the sender cannot put a newer frame
        // ahead of the snapshot this subscriber is about to get (nothing else holds a new subscriber's lock). Any
        // other write waits until the core is free: a slow reader must never hold the core.
        let early = if first { Some(lock(&out)) } else { None };
        drop(c);
        let wants_reply = reply.as_ref().is_some_and(|r| subscribe || matches!(r, Evt::Pong { .. } | Evt::Record { .. }));
        if wants_reply || route.is_some() {
            let mut w = early.unwrap_or_else(|| lock(&out));
            if let Some(r) = reply.filter(|_| wants_reply) {
                if svc::write_frame(&mut *w, &r.encode()).is_err() {
                    break;
                }
            }
            // a new subscriber learns the output route at once (it follows the state it just got)
            if let Some((headphones, reason)) = route {
                if svc::write_frame(&mut *w, &Evt::Route { headphones, reason }.encode()).is_err() {
                    break;
                }
            }
        }
    }
    // the connection that started a recording owns it: if it drops (the app crashed or was killed) the take is saved
    // and the microphone closed
    stop_recording(&core, Some(conn), false);
    // and the playback it started stops: an app that quits must not leave its music or radio stream running
    lock(&core).client_gone(conn);
}

/// Wakes the state ticker: a subscriber arrived, a command ran or the engine signalled a change.
static WAKE: std::sync::Condvar = std::sync::Condvar::new();
/// Wakes the sender thread: broadcast frames are waiting.
static OUTBOX: std::sync::Condvar = std::sync::Condvar::new();

/// Writes queued broadcasts to the subscribers, outside the core lock. One thread, so frames keep their order.
fn sender(core: Arc<Mutex<Core>>) {
    loop {
        let (frames, targets) = {
            let mut c = lock(&core);
            while c.outbox.is_empty() {
                c = wait(&OUTBOX, c);
            }
            let frames = std::mem::take(&mut c.outbox);
            let targets: Vec<(u32, Arc<Mutex<UnixStream>>, u64)> = c.clients.iter().map(|k| (k.id, k.out.clone(), k.since)).collect();
            (frames, targets)
        };
        let mut gone = Vec::new();
        for (id, out, since) in targets {
            let mut w = lock(&out);
            if due(&frames, since).any(|b| svc::write_frame(&mut *w, b).is_err()) {
                gone.push(id);
            }
        }
        if !gone.is_empty() {
            lock(&core).clients.retain(|c| !gone.contains(&c.id));
        }
    }
}

/// Turns the engine's wake-ups (a song or state change, an error) into a ticker wake-up. Blocks in `poll`: no
/// wake-ups of its own.
fn engine_bridge(core: Arc<Mutex<Core>>, fd: std::ffi::c_int) {
    loop {
        let mut p = PollFd { fd, events: 1, revents: 0 };
        // SAFETY: one valid pollfd; the descriptor belongs to the player, which lives as long as the process.
        let n = unsafe { poll(&mut p, 1, -1) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if p.revents & !1 != 0 {
            return;
        }
        let mut c = lock(&core);
        c.player.drain();
        c.dirty = true;
        drop(c);
        WAKE.notify_all();
    }
}

#[repr(C)]
struct PollFd {
    fd: std::ffi::c_int,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: std::ffi::c_ulong, timeout: std::ffi::c_int) -> std::ffi::c_int;
}

fn main() -> std::io::Result<()> {
    let sock = std::env::var("BIFROST_SERVICE_SOCK").unwrap_or_else(|_| "/var/lib/springboard/run/is.olibuijr.audio.sock".into());
    let player = AudioPlayer::open_device()?;
    let fd = player.fd();
    let core = Arc::new(Mutex::new(Core::new(player)));
    // socket activation: bifrostd binds the socket and starts this process on the first connection
    let listener = svc::listen(std::path::Path::new(&sock))?;
    eprintln!("audio.player: {CLIENT} {VERSION} listening on {sock}");

    {
        let core = core.clone();
        std::thread::spawn(move || sender(core));
    }
    {
        let core = core.clone();
        std::thread::spawn(move || engine_bridge(core, fd));
    }
    // pushes state to subscribers when the song, the state or the second changes, and the meter while recording. It
    // runs (five times a second) only while something plays or records; otherwise it parks until a command, the
    // engine or a new subscriber gives it a reason.
    let ticker = core.clone();
    std::thread::spawn(move || {
        let mut last: Option<(Snapshot, u64)> = None;
        loop {
            let mut c = lock(&ticker);
            while c.clients.is_empty() || !(c.dirty || c.recorder.is_some() || c.player.snapshot().playing) {
                c = wait(&WAKE, c);
            }
            c.dirty = false;
            let snap = snapshot(c.player.snapshot(), c.muted);
            let active = snap.playing || c.recorder.is_some();
            let key = (snap.position_ms / 1000.0) as u64;
            let moved = last.as_ref().is_none_or(|(s, k)| Snapshot { position_ms: 0.0, ..s.clone() } != Snapshot { position_ms: 0.0, ..snap.clone() } || *k != key);
            if moved {
                last = Some((snap.clone(), key));
                c.send_all(&Evt::State(snap));
            }
            let mut too_long = false;
            if let Some(a) = &c.recorder {
                too_long = a.recorder.elapsed_ms() >= MAX_RECORD_MS;
                let evt = c.record_state();
                c.send_all(&evt);
            }
            drop(c);
            if too_long {
                // a take that reaches the cap is saved and ended, whoever started it
                stop_recording(&ticker, None, false);
            }
            if active {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    });

    let mut next_conn = 1u32;
    for stream in listener.incoming().flatten() {
        let core = core.clone();
        let conn = next_conn;
        next_conn = next_conn.wrapping_add(1).max(1);
        std::thread::spawn(move || serve(core, stream, conn));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_needs_the_apps_saved_microphone_grant() {
        let grants = "springboard-privacy-grants-v1\nmedia\tis.a\tmicrophone\t1\nmedia\tis.b\tmicrophone\t0\nmedia\tis.c\tcamera\t1\n";
        assert!(microphone_granted(grants, "is.a"));
        assert!(!microphone_granted(grants, "is.b"), "a denial refuses");
        assert!(!microphone_granted(grants, "is.c"), "a camera grant is not a microphone grant");
        assert!(!microphone_granted(grants, "is.d"), "not answered refuses");
        assert!(!microphone_granted("", "is.a"), "no grants file refuses");
        assert!(!microphone_granted("other-header\nmedia\tis.a\tmicrophone\t1\n", "is.a"));
        assert!(!microphone_granted(grants, "is"), "a prefix of an id is not the id");
    }

    #[test]
    fn the_recording_app_is_the_data_directory_the_take_lands_in() {
        let root = Path::new("/srv/appdata");
        assert_eq!(record_app(Path::new("/srv/appdata/is.a/Voice Memo.wav"), root).as_deref(), Some("is.a"));
        assert_eq!(record_app(Path::new("/srv/appdata/is.a/sub/x.wav"), root).as_deref(), Some("is.a"));
        assert_eq!(record_app(Path::new("/srv/appdata/x.wav"), root), None, "a file straight in appdata names no app");
        assert_eq!(record_app(Path::new("/elsewhere/is.a/x.wav"), root), None);
    }

    #[test]
    fn check_microphone_reads_the_grants_beside_appdata() {
        let base = std::env::temp_dir().join(format!("audiod-mic-{}", std::process::id()));
        let appdata = base.join("appdata");
        std::fs::create_dir_all(appdata.join("is.yes")).unwrap();
        std::fs::create_dir_all(appdata.join("is.no")).unwrap();
        std::fs::write(base.join(GRANTS_FILE), format!("{GRANTS_HEADER}\nmedia\tis.yes\tmicrophone\t1\n")).unwrap();
        let canon = std::fs::canonicalize(&appdata).unwrap();
        assert!(check_microphone(&canon.join("is.yes").join("m.wav"), &appdata).is_ok());
        let refused = check_microphone(&canon.join("is.no").join("m.wav"), &appdata).unwrap_err();
        assert!(refused.contains("is.no"), "{refused}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn wire_track_with_url_is_live_and_without_is_a_file() {
        let live = track_in(&WireTrack { id: 1, path: String::new(), url: Some("http://x/stream".into()) });
        assert_eq!(live.url.as_deref(), Some("http://x/stream"));
        let file = track_in(&WireTrack { id: 2, path: "/m/a.mp3".into(), url: None });
        assert_eq!(file.path, PathBuf::from("/m/a.mp3"));
        assert!(file.url.is_none());
    }

    #[test]
    fn a_poisoned_core_lock_still_hands_out_the_core() {
        // a client handler that panics while holding the core must not take the audio service down with it
        let core: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(vec![1]));
        let poisoner = core.clone();
        let _ = std::thread::spawn(move || {
            let _held = poisoner.lock().unwrap();
            panic!("client handler panicked while holding the core");
        })
        .join();
        assert!(core.is_poisoned());
        lock(&core).push(2);
        assert_eq!(*lock(&core), vec![1, 2]);
    }

    #[test]
    fn track_round_trips_through_the_wire() {
        let w = WireTrack { id: 3, path: "/m/b.flac".into(), url: None };
        let back = track_out(&track_in(&w));
        assert_eq!((back.id, back.path.as_str(), back.url), (3, "/m/b.flac", None));
    }

    /// A fresh directory tree: `<tmp>/root/appdata/app/` and `<tmp>/outside/`.
    fn tree(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("audiod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let appdata = base.join("root").join("appdata");
        std::fs::create_dir_all(appdata.join("app")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        (base.clone(), appdata, base.join("outside"))
    }

    #[test]
    fn record_paths_must_stay_inside_the_apps_data_directory() {
        let (base, appdata, outside) = tree("paths");
        let ok = appdata.join("app").join("memo.wav");
        assert_eq!(validate_record_path(ok.to_str().unwrap(), &appdata).unwrap(), std::fs::canonicalize(appdata.join("app")).unwrap().join("memo.wav"));
        // relative, wrong extension, parent hops, outside the area
        assert!(validate_record_path("memo.wav", &appdata).is_err());
        assert!(validate_record_path(appdata.join("app").join("memo.txt").to_str().unwrap(), &appdata).is_err());
        assert!(validate_record_path(&format!("{}/app/../../outside/x.wav", appdata.display()), &appdata).is_err());
        assert!(validate_record_path(outside.join("x.wav").to_str().unwrap(), &appdata).is_err());
        assert!(validate_record_path("/etc/passwd.wav", &appdata).is_err());
        // a directory that is a link out of the area is resolved and refused
        std::os::unix::fs::symlink(&outside, appdata.join("app").join("escape")).unwrap();
        assert!(validate_record_path(appdata.join("app").join("escape").join("x.wav").to_str().unwrap(), &appdata).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_wav_is_written_whole_and_never_through_a_link() {
        let (base, appdata, outside) = tree("write");
        let target = appdata.join("app").join("a.wav");
        write_wav_atomic(&target, b"RIFFdata").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"RIFFdata");
        assert!(!appdata.join("app").join("a.wav.tmp").exists());
        // a link at the target name is refused and what it points at is untouched
        let victim = outside.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let link = appdata.join("app").join("b.wav");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(write_wav_atomic(&link, b"RIFFdata").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        // a link at the temporary name is refused too
        let c = appdata.join("app").join("c.wav");
        std::os::unix::fs::symlink(&victim, appdata.join("app").join("c.wav.tmp")).unwrap();
        assert!(write_wav_atomic(&c, b"RIFFdata").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        // a directory that is gone: an error, the caller keeps its take
        assert!(write_wav_atomic(&appdata.join("missing").join("d.wav"), b"x").is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_subscriber_gets_only_the_frames_after_it_joined() {
        let frames = vec![(1, vec![1u8]), (2, vec![2]), (3, vec![3])];
        assert_eq!(due(&frames, 0).count(), 3);
        assert_eq!(due(&frames, 2).cloned().collect::<Vec<_>>(), vec![vec![3u8]]);
        assert_eq!(due(&frames, 3).count(), 0);
    }

    fn core() -> Core {
        Core::new(AudioPlayer::new(Box::new(springboard::media::audio_out::NullOut::new())).unwrap())
    }

    fn queue_cmd() -> Cmd {
        let tracks = vec![WireTrack { id: 1, path: "/nonexistent/a.mp3".into(), url: None }, WireTrack { id: 2, path: "/nonexistent/b.mp3".into(), url: None }];
        Cmd::PlayQueue { tracks, start: 0, shuffle: false, seed: 1 }
    }

    #[test]
    fn playback_ends_when_the_connection_that_started_it_closes() {
        let mut c = core();
        c.apply(queue_cmd());
        c.playback_owner = Some(7);
        assert_eq!(c.player.snapshot().queue_len, 2);
        // another connection (the shell) going away leaves the owner's playback alone
        c.client_gone(3);
        assert_eq!(c.player.snapshot().queue_len, 2, "only the owner's exit ends playback");
        assert_eq!(c.playback_owner, Some(7));
        c.client_gone(7);
        let s = c.player.snapshot();
        assert_eq!((s.queue_len, s.playing, s.track.is_none()), (0, false, true), "{s:?}");
        assert_eq!(c.playback_owner, None);
        c.player.stop();
    }

    #[test]
    fn a_closing_connection_is_dropped_from_the_subscribers() {
        let mut c = core();
        let (a, _b) = UnixStream::pair().unwrap();
        c.clients.push(Client { id: 4, out: Arc::new(Mutex::new(a)), since: 0 });
        c.client_gone(4);
        assert!(c.clients.is_empty());
        c.player.stop();
    }

    #[test]
    fn take_length_follows_samples_channels_and_rate() {
        let take = Take { format: springboard::media::capture::pcm16(48_000, 1), samples: vec![0; 96_000] };
        assert_eq!(take_ms(&take), 2000);
    }
}
