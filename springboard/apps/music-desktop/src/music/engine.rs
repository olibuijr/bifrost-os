//! The playback engine as the app sees it: the audio service's client behind the
//! types the UI was written against (`Track`, `Snapshot`, `Phase`, `AudioPlayer`).

use bifrost_app::audio::Client;
use bifrost_app::proto::svc::audio as wire;
use std::path::PathBuf;

pub use springboard::media::queue::Repeat;

/// One queued song, or a live stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// the library's id for the song (a station's id for a stream)
    pub id: u64,
    pub path: PathBuf,
    /// a live track has no file, no length and no seeking
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

    fn from_wire(t: &wire::Track) -> Self {
        Track { id: t.id, path: PathBuf::from(&t.path), url: t.url.clone() }
    }

    fn to_wire(&self) -> wire::Track {
        wire::Track { id: self.id, path: self.path.to_string_lossy().into_owned(), url: self.url.clone() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Buffering,
    Live,
    /// the n-th attempt to get the connection back
    Reconnecting(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveStatus {
    pub phase: Phase,
    pub title: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub track: Option<Track>,
    pub index: Option<usize>,
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub queue_len: usize,
    pub volume: f32,
    pub error: Option<String>,
    pub live: Option<LiveStatus>,
}

fn repeat(r: wire::Repeat) -> Repeat {
    match r {
        wire::Repeat::Off => Repeat::Off,
        wire::Repeat::All => Repeat::All,
        wire::Repeat::One => Repeat::One,
    }
}

fn wire_repeat(r: Repeat) -> wire::Repeat {
    match r {
        Repeat::Off => wire::Repeat::Off,
        Repeat::All => wire::Repeat::All,
        Repeat::One => wire::Repeat::One,
    }
}

fn phase(p: u32) -> Phase {
    match p {
        0 => Phase::Buffering,
        1 => Phase::Live,
        n => Phase::Reconnecting(n - 1),
    }
}

/// The audio service's client, shaped like the old in-process player.
pub struct AudioPlayer {
    client: Client,
}

impl AudioPlayer {
    pub fn new(client: Client) -> Self {
        AudioPlayer { client }
    }

    /// Reads what the service sent and keeps the connection alive.
    pub fn drain(&mut self, now: f64) {
        self.client.pump();
        self.client.tick(now);
    }

    pub fn next_wake(&self, now: f64) -> Option<f64> {
        self.client.next_wake(now)
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.client.snapshot();
        Snapshot {
            track: s.track.as_ref().map(Track::from_wire),
            index: s.index.map(|i| i as usize),
            playing: s.playing,
            position_ms: s.position_ms,
            duration_ms: s.duration_ms,
            shuffle: s.shuffle,
            repeat: repeat(s.repeat),
            queue_len: s.queue_len as usize,
            volume: s.volume,
            error: s.error.clone(),
            live: s.live.as_ref().map(|l| LiveStatus { phase: phase(l.phase), title: l.title.clone(), error: l.error.clone() }),
        }
    }

    pub fn play_queue(&mut self, tracks: Vec<Track>, start: usize, shuffle: bool, seed: u64) {
        self.client.play_queue(tracks.iter().map(Track::to_wire).collect(), start as u32, shuffle, seed);
    }

    pub fn pause(&mut self) {
        self.client.pause();
    }

    pub fn toggle(&mut self) {
        self.client.toggle();
    }

    pub fn next(&mut self) {
        self.client.next();
    }

    pub fn prev(&mut self) {
        self.client.prev();
    }

    pub fn jump_to(&mut self, index: usize) {
        self.client.send(wire::Cmd::JumpTo(index as u32));
    }

    pub fn seek_ms(&mut self, ms: f64) {
        self.client.seek_ms(ms);
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.client.set_repeat(wire_repeat(r));
    }

    pub fn set_volume(&mut self, v: f32) {
        self.client.set_volume(v);
    }

    /// Crossfades one song into the next over `ms` milliseconds; 0 plays gapless. The audio service keeps it until it
    /// restarts.
    pub fn set_crossfade_ms(&mut self, ms: u32) {
        self.client.set_crossfade_ms(ms);
    }
}
