//! The media player service (`is.olibuijr.mediaplayer` on the audio server): decodes and plays music and records voice memos as a client of bifrost-audiod. Music, Podcasts,
//! Voice Memos and the shell talk to it over `BIFROST_SERVICE_SOCK` (frames from
//! `bifrost_proto::svc::audio`), so closing or updating an app never cuts the sound.

use bifrost_proto::svc::audio::{Cmd, Evt, Live, Repeat as WireRepeat, Snapshot, Track as WireTrack};
use bifrost_proto::svc;
use springboard::media::audioplayer::{AudioPlayer, Snapshot as PlayerSnapshot, Track};
use springboard::media::capture::{wav_bytes, Recorder};
use springboard::media::queue::Repeat;
use springboard::media::stream::Phase;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
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

struct Client {
    stream: UnixStream,
    subscribed: bool,
}

struct Core {
    player: AudioPlayer,
    recorder: Option<(Recorder, PathBuf)>,
    muted: bool,
    clients: Vec<Client>,
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

impl Core {
    fn state(&self) -> Evt {
        Evt::State(snapshot(self.player.snapshot(), self.muted))
    }

    fn record_state(&self) -> Evt {
        match &self.recorder {
            Some((r, _)) => Evt::Record { active: true, elapsed_ms: r.elapsed_ms(), level: r.level(), error: r.error() },
            None => Evt::Record { active: false, elapsed_ms: 0, level: 0.0, error: None },
        }
    }

    fn send_all(&mut self, evt: &Evt) {
        let bytes = evt.encode();
        self.clients.retain_mut(|c| !c.subscribed || svc::write_frame(&mut c.stream, &bytes).is_ok());
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
            Cmd::SetRate(r) => self.player.set_rate(r),
            Cmd::SetMuted(m) => {
                self.muted = m;
                self.player.set_muted(m);
            }
            Cmd::Stop => self.player.pause(),
            Cmd::RecordStart { path } => {
                if self.recorder.is_none() {
                    match Recorder::start() {
                        Ok(r) => self.recorder = Some((r, PathBuf::from(path))),
                        Err(e) => return Some(Evt::Record { active: false, elapsed_ms: 0, level: 0.0, error: Some(e.to_string()) }),
                    }
                }
                let evt = self.record_state();
                self.send_all(&evt);
                return None;
            }
            Cmd::RecordStop => {
                if let Some((r, path)) = self.recorder.take() {
                    let take = r.stop();
                    let ms = take.samples.len() as u64 / take.format.channels.max(1) as u64 * 1000 / take.format.sample_rate.max(1.0) as u64;
                    let evt = match std::fs::write(&path, wav_bytes(&take.format, &take.samples)) {
                        Ok(()) => Evt::RecordDone { path: path.to_string_lossy().into_owned(), duration_ms: ms },
                        Err(e) => Evt::Record { active: false, elapsed_ms: ms, level: 0.0, error: Some(e.to_string()) },
                    };
                    self.send_all(&evt);
                }
                let evt = self.record_state();
                self.send_all(&evt);
                return None;
            }
        }
        let evt = self.state();
        self.send_all(&evt);
        None
    }
}

fn serve(core: Arc<Mutex<Core>>, mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut writer = Some(writer);
    // the connection that started a recording owns it: if it drops (the app crashed or was killed) the take is saved and the microphone closed
    let mut recording = false;
    while let Ok(frame) = svc::read_frame(&mut stream) {
        let Ok(cmd) = Cmd::decode(&frame) else { continue };
        let subscribe = cmd == Cmd::Subscribe;
        let mut c = lock(&core);
        let free = c.recorder.is_none();
        let (starts, stops) = (matches!(cmd, Cmd::RecordStart { .. }), cmd == Cmd::RecordStop);
        let reply = c.apply(cmd);
        if starts && free && c.recorder.is_some() {
            recording = true;
        } else if stops && recording {
            recording = false;
        }
        if subscribe {
            if let Some(w) = writer.take() {
                c.clients.push(Client { stream: w, subscribed: true });
                CLIENTS.notify_all();
            }
        }
        if let Some(r) = reply {
            if subscribe || matches!(r, Evt::Pong { .. } | Evt::Record { .. }) {
                if svc::write_frame(&mut stream, &r.encode()).is_err() {
                    break;
                }
            }
        }
    }
    if recording {
        lock(&core).apply(Cmd::RecordStop);
    }
}

/// Wakes the state ticker when the first client arrives (it parks while nobody listens).
static CLIENTS: std::sync::Condvar = std::sync::Condvar::new();

fn main() -> std::io::Result<()> {
    let sock = std::env::var("BIFROST_SERVICE_SOCK").unwrap_or_else(|_| "/var/lib/springboard/run/is.olibuijr.audio.sock".into());
    let player = AudioPlayer::open_device()?;
    let core = Arc::new(Mutex::new(Core { player, recorder: None, muted: true, clients: Vec::new() }));
    // socket activation: bifrostd binds the socket and starts this process on the first connection
    let listener = svc::listen(std::path::Path::new(&sock))?;
    eprintln!("audio.player: {CLIENT} {VERSION} listening on {sock}");

    // pushes state to subscribers when the song, the state or the second changes, and the meter while recording
    let ticker = core.clone();
    std::thread::spawn(move || {
        let mut last: Option<(Snapshot, u64)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let mut c = lock(&ticker);
            while c.clients.is_empty() {
                c = wait(&CLIENTS, c);
            }
            let snap = snapshot(c.player.snapshot(), c.muted);
            let key = (snap.position_ms / 1000.0) as u64;
            let moved = last.as_ref().is_none_or(|(s, k)| Snapshot { position_ms: 0.0, ..s.clone() } != Snapshot { position_ms: 0.0, ..snap.clone() } || *k != key);
            if moved {
                last = Some((snap.clone(), key));
                c.send_all(&Evt::State(snap));
            }
            if c.recorder.is_some() {
                let evt = c.record_state();
                c.send_all(&evt);
            }
        }
    });

    for stream in listener.incoming().flatten() {
        let core = core.clone();
        std::thread::spawn(move || serve(core, stream));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
