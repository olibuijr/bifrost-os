//! Wire format between SpringBoard / apps and a supervised service (docs/services.md).
//!
//! Transport: a `SOCK_STREAM` unix socket at `<root>/run/<id>.sock`. A frame is `u32 len`
//! (little-endian, at most `MAX_FRAME`) followed by `len` bytes: `u32 tag` and the fields.
//! Strings are `u32 len` + UTF-8, options are a `u8` flag then the value. Unknown tags decode
//! to `Err(DecodeError::UnknownTag)` so both sides can skip them.

use std::io::{self, Read, Write};

pub use crate::DecodeError;

pub const MAX_FRAME: usize = 256 * 1024;

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(payload.len() + 4);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut v = vec![0u8; len];
    r.read_exact(&mut v)?;
    Ok(v)
}

/// Appends fields to a payload.
#[derive(Default)]
pub struct Enc(pub Vec<u8>);

impl Enc {
    pub fn tag(tag: u32) -> Self {
        Enc(tag.to_le_bytes().to_vec())
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn bool(self, v: bool) -> Self {
        self.u8(v as u8)
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(self, v: f32) -> Self {
        self.u32(v.to_bits())
    }
    pub fn f64(self, v: f64) -> Self {
        self.u64(v.to_bits())
    }
    pub fn str(self, v: &str) -> Self {
        let mut e = self.u32(v.len() as u32);
        e.0.extend_from_slice(v.as_bytes());
        e
    }
    pub fn opt_str(self, v: &Option<String>) -> Self {
        match v {
            Some(s) => self.bool(true).str(s),
            None => self.bool(false),
        }
    }
    pub fn opt_f64(self, v: Option<f64>) -> Self {
        match v {
            Some(x) => self.bool(true).f64(x),
            None => self.bool(false),
        }
    }
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// Reads fields from a payload.
pub struct Dec<'a>(&'a [u8]);

impl<'a> Dec<'a> {
    /// Splits the tag off the payload.
    pub fn new(payload: &'a [u8]) -> Result<(u32, Dec<'a>), DecodeError> {
        let tag = payload.get(..4).ok_or(DecodeError::Truncated)?;
        Ok((u32::from_le_bytes(tag.try_into().unwrap()), Dec(&payload[4..])))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_bits(self.u64()?))
    }
    pub fn str(&mut self) -> Result<String, DecodeError> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| DecodeError::BadUtf8)
    }
    pub fn opt_str(&mut self) -> Result<Option<String>, DecodeError> {
        if self.bool()? { self.str().map(Some) } else { Ok(None) }
    }
    pub fn opt_f64(&mut self) -> Result<Option<f64>, DecodeError> {
        if self.bool()? { self.f64().map(Some) } else { Ok(None) }
    }
}

/// Messages every service answers, on top of its own.
pub mod common {
    use super::*;

    pub const PING: u32 = 1;
    pub const PONG: u32 = 0x101;

    pub fn ping() -> Vec<u8> {
        Enc::tag(PING).done()
    }

    /// The reply: the service's id and version, so the supervisor knows which build answered.
    pub fn pong(id: &str, version: &str) -> Vec<u8> {
        Enc::tag(PONG).str(id).str(version).done()
    }

    pub fn parse_pong(payload: &[u8]) -> Result<(String, String), DecodeError> {
        let (tag, mut d) = Dec::new(payload)?;
        if tag != PONG {
            return Err(DecodeError::UnknownTag(tag));
        }
        Ok((d.str()?, d.str()?))
    }
}

/// The audio service (`is.olibuijr.audio`): the one owner of the speaker and the microphone.
pub mod audio {
    use super::*;

    /// One queued song or live stream.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Track {
        pub id: u64,
        pub path: String,
        pub url: Option<String>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Repeat {
        Off = 0,
        All = 1,
        One = 2,
    }

    /// Where a live stream is: 0 buffering, 1 live, n+2 the n-th reconnect.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Live {
        pub phase: u32,
        pub title: Option<String>,
        pub error: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Snapshot {
        pub track: Option<Track>,
        pub index: Option<u32>,
        pub playing: bool,
        pub position_ms: f64,
        pub duration_ms: Option<f64>,
        pub shuffle: bool,
        pub repeat: Repeat,
        pub queue_len: u32,
        pub volume: f32,
        pub muted: bool,
        pub error: Option<String>,
        pub live: Option<Live>,
    }

    /// Client -> audiod.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Cmd {
        Ping,
        /// send `State` now and on every change (position ticks once a second while playing)
        Subscribe,
        PlayQueue { tracks: Vec<Track>, start: u32, shuffle: bool, seed: u64 },
        Play,
        Pause,
        Toggle,
        Next,
        Prev,
        JumpTo(u32),
        SeekMs(f64),
        SetShuffle { on: bool, seed: u64 },
        SetRepeat(Repeat),
        SetVolume(f32),
        SetMuted(bool),
        Stop,
        /// records the microphone into `path` (WAV); one recording at a time, explicit user action only
        RecordStart { path: String },
        RecordStop,
    }

    /// audiod -> client.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Evt {
        Pong { id: String, version: String },
        State(Snapshot),
        Record { active: bool, elapsed_ms: u64, level: f32, error: Option<String> },
        /// a recording was written
        RecordDone { path: String, duration_ms: u64 },
    }

    mod t {
        pub const SUBSCRIBE: u32 = 2;
        pub const PLAY_QUEUE: u32 = 3;
        pub const PLAY: u32 = 4;
        pub const PAUSE: u32 = 5;
        pub const TOGGLE: u32 = 6;
        pub const NEXT: u32 = 7;
        pub const PREV: u32 = 8;
        pub const JUMP_TO: u32 = 9;
        pub const SEEK: u32 = 10;
        pub const SET_SHUFFLE: u32 = 11;
        pub const SET_REPEAT: u32 = 12;
        pub const SET_VOLUME: u32 = 13;
        pub const SET_MUTED: u32 = 14;
        pub const STOP: u32 = 15;
        pub const RECORD_START: u32 = 16;
        pub const RECORD_STOP: u32 = 17;
        pub const STATE: u32 = 0x102;
        pub const RECORD: u32 = 0x103;
        pub const RECORD_DONE: u32 = 0x104;
    }

    fn repeat(v: u8) -> Result<Repeat, DecodeError> {
        match v {
            0 => Ok(Repeat::Off),
            1 => Ok(Repeat::All),
            2 => Ok(Repeat::One),
            _ => Err(DecodeError::BadValue),
        }
    }

    fn put_track(e: Enc, t: &Track) -> Enc {
        e.u64(t.id).str(&t.path).opt_str(&t.url)
    }

    fn get_track(d: &mut Dec) -> Result<Track, DecodeError> {
        Ok(Track { id: d.u64()?, path: d.str()?, url: d.opt_str()? })
    }

    impl Cmd {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Cmd::Ping => Enc::tag(common::PING),
                Cmd::Subscribe => Enc::tag(t::SUBSCRIBE),
                Cmd::PlayQueue { tracks, start, shuffle, seed } => {
                    let mut e = Enc::tag(t::PLAY_QUEUE).u32(tracks.len() as u32);
                    for tr in tracks {
                        e = put_track(e, tr);
                    }
                    e.u32(*start).bool(*shuffle).u64(*seed)
                }
                Cmd::Play => Enc::tag(t::PLAY),
                Cmd::Pause => Enc::tag(t::PAUSE),
                Cmd::Toggle => Enc::tag(t::TOGGLE),
                Cmd::Next => Enc::tag(t::NEXT),
                Cmd::Prev => Enc::tag(t::PREV),
                Cmd::JumpTo(i) => Enc::tag(t::JUMP_TO).u32(*i),
                Cmd::SeekMs(ms) => Enc::tag(t::SEEK).f64(*ms),
                Cmd::SetShuffle { on, seed } => Enc::tag(t::SET_SHUFFLE).bool(*on).u64(*seed),
                Cmd::SetRepeat(r) => Enc::tag(t::SET_REPEAT).u8(*r as u8),
                Cmd::SetVolume(v) => Enc::tag(t::SET_VOLUME).f32(*v),
                Cmd::SetMuted(m) => Enc::tag(t::SET_MUTED).bool(*m),
                Cmd::Stop => Enc::tag(t::STOP),
                Cmd::RecordStart { path } => Enc::tag(t::RECORD_START).str(path),
                Cmd::RecordStop => Enc::tag(t::RECORD_STOP),
            }
            .done()
        }

        pub fn decode(payload: &[u8]) -> Result<Cmd, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PING => Cmd::Ping,
                t::SUBSCRIBE => Cmd::Subscribe,
                t::PLAY_QUEUE => {
                    let n = d.u32()? as usize;
                    let mut tracks = Vec::with_capacity(n.min(4096));
                    for _ in 0..n {
                        tracks.push(get_track(&mut d)?);
                    }
                    Cmd::PlayQueue { tracks, start: d.u32()?, shuffle: d.bool()?, seed: d.u64()? }
                }
                t::PLAY => Cmd::Play,
                t::PAUSE => Cmd::Pause,
                t::TOGGLE => Cmd::Toggle,
                t::NEXT => Cmd::Next,
                t::PREV => Cmd::Prev,
                t::JUMP_TO => Cmd::JumpTo(d.u32()?),
                t::SEEK => Cmd::SeekMs(d.f64()?),
                t::SET_SHUFFLE => Cmd::SetShuffle { on: d.bool()?, seed: d.u64()? },
                t::SET_REPEAT => Cmd::SetRepeat(repeat(d.u8()?)?),
                t::SET_VOLUME => Cmd::SetVolume(d.f32()?),
                t::SET_MUTED => Cmd::SetMuted(d.bool()?),
                t::STOP => Cmd::Stop,
                t::RECORD_START => Cmd::RecordStart { path: d.str()? },
                t::RECORD_STOP => Cmd::RecordStop,
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    impl Evt {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Evt::Pong { id, version } => return common::pong(id, version),
                Evt::State(s) => {
                    let mut e = Enc::tag(t::STATE);
                    e = match &s.track {
                        Some(tr) => put_track(e.bool(true), tr),
                        None => e.bool(false),
                    };
                    e = match s.index {
                        Some(i) => e.bool(true).u32(i),
                        None => e.bool(false),
                    };
                    e = e.bool(s.playing).f64(s.position_ms).opt_f64(s.duration_ms).bool(s.shuffle).u8(s.repeat as u8);
                    e = e.u32(s.queue_len).f32(s.volume).bool(s.muted).opt_str(&s.error);
                    match &s.live {
                        Some(l) => e.bool(true).u32(l.phase).opt_str(&l.title).opt_str(&l.error),
                        None => e.bool(false),
                    }
                }
                Evt::Record { active, elapsed_ms, level, error } => Enc::tag(t::RECORD).bool(*active).u64(*elapsed_ms).f32(*level).opt_str(error),
                Evt::RecordDone { path, duration_ms } => Enc::tag(t::RECORD_DONE).str(path).u64(*duration_ms),
            }
            .done()
        }

        pub fn decode(payload: &[u8]) -> Result<Evt, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PONG => Evt::Pong { id: d.str()?, version: d.str()? },
                t::STATE => {
                    let track = if d.bool()? { Some(get_track(&mut d)?) } else { None };
                    let index = if d.bool()? { Some(d.u32()?) } else { None };
                    let (playing, position_ms, duration_ms, shuffle) = (d.bool()?, d.f64()?, d.opt_f64()?, d.bool()?);
                    let rep = repeat(d.u8()?)?;
                    let (queue_len, volume, muted, error) = (d.u32()?, d.f32()?, d.bool()?, d.opt_str()?);
                    let live = if d.bool()? { Some(Live { phase: d.u32()?, title: d.opt_str()?, error: d.opt_str()? }) } else { None };
                    Evt::State(Snapshot { track, index, playing, position_ms, duration_ms, shuffle, repeat: rep, queue_len, volume, muted, error, live })
                }
                t::RECORD => Evt::Record { active: d.bool()?, elapsed_ms: d.u64()?, level: d.f32()?, error: d.opt_str()? },
                t::RECORD_DONE => Evt::RecordDone { path: d.str()?, duration_ms: d.u64()? },
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn commands_round_trip() {
            let cmds = vec![
                Cmd::Ping,
                Cmd::Subscribe,
                Cmd::PlayQueue { tracks: vec![Track { id: 7, path: "/a.mp3".into(), url: None }, Track { id: 9, path: String::new(), url: Some("https://r".into()) }], start: 1, shuffle: true, seed: 42 },
                Cmd::JumpTo(3),
                Cmd::SeekMs(1234.5),
                Cmd::SetRepeat(Repeat::One),
                Cmd::SetVolume(0.5),
                Cmd::RecordStart { path: "/m.wav".into() },
            ];
            for c in cmds {
                assert_eq!(Cmd::decode(&c.encode()), Ok(c));
            }
        }

        #[test]
        fn state_round_trips_and_unknown_tags_are_skippable() {
            let s = Snapshot {
                track: Some(Track { id: 1, path: "/x".into(), url: None }),
                index: Some(0),
                playing: true,
                position_ms: 10.0,
                duration_ms: Some(200.0),
                shuffle: false,
                repeat: Repeat::All,
                queue_len: 4,
                volume: 0.6,
                muted: false,
                error: None,
                live: Some(Live { phase: 1, title: Some("t".into()), error: None }),
            };
            let e = Evt::State(s);
            assert_eq!(Evt::decode(&e.encode()), Ok(e));
            assert_eq!(Cmd::decode(&Enc::tag(0x7fff).done()), Err(DecodeError::UnknownTag(0x7fff)));
            assert_eq!(Cmd::decode(&[1]), Err(DecodeError::Truncated));
        }

        #[test]
        fn frames_round_trip() {
            let mut buf = Vec::new();
            write_frame(&mut buf, &Cmd::Play.encode()).unwrap();
            let got = read_frame(&mut buf.as_slice()).unwrap();
            assert_eq!(Cmd::decode(&got), Ok(Cmd::Play));
        }
    }
}

/// The network service (`is.olibuijr.net`): the one owner of Wi-Fi (supplicant, DHCP, routes).
pub mod net {
    use super::*;

    /// 0 off, 1 starting, 2 searching, 3 connecting, 4 connected.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Ap {
        pub ssid: String,
        pub bssid: String,
        pub signal: i32,
        pub secure: bool,
        pub enterprise: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Status {
        pub state: u8,
        pub ssid: Option<String>,
        pub ip: Option<String>,
        pub router: Option<String>,
        pub rssi: Option<i32>,
        pub scan: Vec<Ap>,
        /// saved networks, highest priority first
        pub saved: Vec<String>,
        /// per-network problem ("Incorrect Password", "No Internet Connection")
        pub notes: Vec<(String, String)>,
        pub error: Option<String>,
    }

    /// Client -> netd.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Cmd {
        Ping,
        /// send `Status` now and on every change
        Subscribe,
        Scan,
        Join { ssid: String, psk: Option<String> },
        Select(String),
        Forget(String),
        SetAutoJoin { ssid: String, on: bool },
        Move { ssid: String, up: bool },
        Enable(bool),
    }

    /// netd -> client.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Evt {
        Pong { id: String, version: String },
        Status(Status),
    }

    mod t {
        pub const SUBSCRIBE: u32 = 2;
        pub const SCAN: u32 = 3;
        pub const JOIN: u32 = 4;
        pub const SELECT: u32 = 5;
        pub const FORGET: u32 = 6;
        pub const SET_AUTO_JOIN: u32 = 7;
        pub const MOVE: u32 = 8;
        pub const ENABLE: u32 = 9;
        pub const STATUS: u32 = 0x102;
    }

    impl Cmd {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Cmd::Ping => Enc::tag(common::PING),
                Cmd::Subscribe => Enc::tag(t::SUBSCRIBE),
                Cmd::Scan => Enc::tag(t::SCAN),
                Cmd::Join { ssid, psk } => Enc::tag(t::JOIN).str(ssid).opt_str(psk),
                Cmd::Select(s) => Enc::tag(t::SELECT).str(s),
                Cmd::Forget(s) => Enc::tag(t::FORGET).str(s),
                Cmd::SetAutoJoin { ssid, on } => Enc::tag(t::SET_AUTO_JOIN).str(ssid).bool(*on),
                Cmd::Move { ssid, up } => Enc::tag(t::MOVE).str(ssid).bool(*up),
                Cmd::Enable(on) => Enc::tag(t::ENABLE).bool(*on),
            }
            .done()
        }

        pub fn decode(payload: &[u8]) -> Result<Cmd, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PING => Cmd::Ping,
                t::SUBSCRIBE => Cmd::Subscribe,
                t::SCAN => Cmd::Scan,
                t::JOIN => Cmd::Join { ssid: d.str()?, psk: d.opt_str()? },
                t::SELECT => Cmd::Select(d.str()?),
                t::FORGET => Cmd::Forget(d.str()?),
                t::SET_AUTO_JOIN => Cmd::SetAutoJoin { ssid: d.str()?, on: d.bool()? },
                t::MOVE => Cmd::Move { ssid: d.str()?, up: d.bool()? },
                t::ENABLE => Cmd::Enable(d.bool()?),
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    impl Evt {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Evt::Pong { id, version } => common::pong(id, version),
                Evt::Status(s) => {
                    let mut e = Enc::tag(t::STATUS).u8(s.state).opt_str(&s.ssid).opt_str(&s.ip).opt_str(&s.router).opt_f64(s.rssi.map(f64::from));
                    e = e.u32(s.scan.len() as u32);
                    for a in &s.scan {
                        e = e.str(&a.ssid).str(&a.bssid).u32(a.signal as u32).bool(a.secure).bool(a.enterprise);
                    }
                    e = e.u32(s.saved.len() as u32);
                    for n in &s.saved {
                        e = e.str(n);
                    }
                    e = e.u32(s.notes.len() as u32);
                    for (k, v) in &s.notes {
                        e = e.str(k).str(v);
                    }
                    e.opt_str(&s.error).done()
                }
            }
        }

        pub fn decode(payload: &[u8]) -> Result<Evt, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PONG => Evt::Pong { id: d.str()?, version: d.str()? },
                t::STATUS => {
                    let (state, ssid, ip, router, rssi) = (d.u8()?, d.opt_str()?, d.opt_str()?, d.opt_str()?, d.opt_f64()?.map(|r| r as i32));
                    let mut scan = Vec::new();
                    for _ in 0..d.u32()? {
                        scan.push(Ap { ssid: d.str()?, bssid: d.str()?, signal: d.u32()? as i32, secure: d.bool()?, enterprise: d.bool()? });
                    }
                    let mut saved = Vec::new();
                    for _ in 0..d.u32()? {
                        saved.push(d.str()?);
                    }
                    let mut notes = Vec::new();
                    for _ in 0..d.u32()? {
                        notes.push((d.str()?, d.str()?));
                    }
                    Evt::Status(Status { state, ssid, ip, router, rssi, scan, saved, notes, error: d.opt_str()? })
                }
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn commands_round_trip() {
            let cmds = vec![
                Cmd::Ping,
                Cmd::Subscribe,
                Cmd::Scan,
                Cmd::Join { ssid: "Home".into(), psk: Some("pw".into()) },
                Cmd::Join { ssid: "Cafe".into(), psk: None },
                Cmd::Select("Home".into()),
                Cmd::Forget("Home".into()),
                Cmd::SetAutoJoin { ssid: "Home".into(), on: false },
                Cmd::Move { ssid: "Home".into(), up: true },
                Cmd::Enable(true),
            ];
            for c in cmds {
                assert_eq!(Cmd::decode(&c.encode()), Ok(c));
            }
        }

        #[test]
        fn status_round_trips_with_negative_rssi() {
            let s = Status {
                state: 4,
                ssid: Some("Home".into()),
                ip: Some("192.168.1.20".into()),
                router: Some("192.168.1.1".into()),
                rssi: Some(-57),
                scan: vec![Ap { ssid: "Home".into(), bssid: "aa:bb".into(), signal: -57, secure: true, enterprise: false }],
                saved: vec!["Home".into(), "Work".into()],
                notes: vec![("Work".into(), "Incorrect Password".into())],
                error: None,
            };
            let e = Evt::Status(s);
            assert_eq!(Evt::decode(&e.encode()), Ok(e));
            assert_eq!(Cmd::decode(&Enc::tag(0x7fff).done()), Err(DecodeError::UnknownTag(0x7fff)));
        }
    }
}

/// The update agent (`is.olibuijr.update`): background checks for OS and app updates,
/// system package installs and the OS download, independent of the shell.
pub mod update {
    use super::*;

    /// What the OS manifest offers.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Offer {
        pub version: String,
        pub build: Option<String>,
        pub sha256: String,
        pub size: u64,
        pub url: String,
        pub notes: Option<String>,
    }

    /// Where the agent is with the OS update.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Phase {
        Idle = 0,
        Checking = 1,
        UpToDate = 2,
        Available = 3,
        Downloading = 4,
        /// `springboard.new` is in place; the shell restarts to install it
        Staged = 5,
        Failed = 6,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct State {
        pub phase: Phase,
        pub offer: Option<Offer>,
        /// download progress 0..=1
        pub fraction: f32,
        /// (id, newer version) of installed apps with an update in the store
        pub apps: Vec<(String, String)>,
        /// ids among `apps` that are system packages
        pub system: Vec<String>,
        pub error: Option<String>,
    }

    /// Client -> updated.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Cmd {
        Ping,
        /// send `State` now and on every change
        Subscribe,
        /// the running shell's version and git stamp, whether to download offers on its own, and an
        /// optional manifest URL override; sent on connect and whenever one of them changes
        Configure { version: String, git: String, auto_download: bool, url: Option<String> },
        /// the conditions the background schedule honours
        Conditions { wifi: bool, low_power: bool, charging: bool },
        /// check now (Settings > Software Update)
        Check,
        /// download the offered OS update
        Download,
    }

    /// updated -> client.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Evt {
        Pong { id: String, version: String },
        State(State),
        /// system packages the agent installed or updated; the shell hot-reloads what they provide
        SystemInstalled(Vec<String>),
    }

    mod t {
        pub const SUBSCRIBE: u32 = 2;
        pub const CONFIGURE: u32 = 3;
        pub const CONDITIONS: u32 = 4;
        pub const CHECK: u32 = 5;
        pub const DOWNLOAD: u32 = 6;
        pub const STATE: u32 = 0x102;
        pub const SYSTEM_INSTALLED: u32 = 0x103;
    }

    fn phase(v: u8) -> Result<Phase, DecodeError> {
        Ok(match v {
            0 => Phase::Idle,
            1 => Phase::Checking,
            2 => Phase::UpToDate,
            3 => Phase::Available,
            4 => Phase::Downloading,
            5 => Phase::Staged,
            6 => Phase::Failed,
            _ => return Err(DecodeError::BadValue),
        })
    }

    impl Cmd {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Cmd::Ping => Enc::tag(common::PING),
                Cmd::Subscribe => Enc::tag(t::SUBSCRIBE),
                Cmd::Configure { version, git, auto_download, url } => Enc::tag(t::CONFIGURE).str(version).str(git).bool(*auto_download).opt_str(url),
                Cmd::Conditions { wifi, low_power, charging } => Enc::tag(t::CONDITIONS).bool(*wifi).bool(*low_power).bool(*charging),
                Cmd::Check => Enc::tag(t::CHECK),
                Cmd::Download => Enc::tag(t::DOWNLOAD),
            }
            .done()
        }

        pub fn decode(payload: &[u8]) -> Result<Cmd, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PING => Cmd::Ping,
                t::SUBSCRIBE => Cmd::Subscribe,
                t::CONFIGURE => Cmd::Configure { version: d.str()?, git: d.str()?, auto_download: d.bool()?, url: d.opt_str()? },
                t::CONDITIONS => Cmd::Conditions { wifi: d.bool()?, low_power: d.bool()?, charging: d.bool()? },
                t::CHECK => Cmd::Check,
                t::DOWNLOAD => Cmd::Download,
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    impl Evt {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Evt::Pong { id, version } => common::pong(id, version),
                Evt::State(s) => {
                    let mut e = Enc::tag(t::STATE).u8(s.phase as u8);
                    e = match &s.offer {
                        Some(o) => e.bool(true).str(&o.version).opt_str(&o.build).str(&o.sha256).u64(o.size).str(&o.url).opt_str(&o.notes),
                        None => e.bool(false),
                    };
                    e = e.f32(s.fraction).u32(s.apps.len() as u32);
                    for (id, v) in &s.apps {
                        e = e.str(id).str(v);
                    }
                    e = e.u32(s.system.len() as u32);
                    for id in &s.system {
                        e = e.str(id);
                    }
                    e.opt_str(&s.error).done()
                }
                Evt::SystemInstalled(ids) => {
                    let mut e = Enc::tag(t::SYSTEM_INSTALLED).u32(ids.len() as u32);
                    for id in ids {
                        e = e.str(id);
                    }
                    e.done()
                }
            }
        }

        pub fn decode(payload: &[u8]) -> Result<Evt, DecodeError> {
            let (tag, mut d) = Dec::new(payload)?;
            Ok(match tag {
                common::PONG => Evt::Pong { id: d.str()?, version: d.str()? },
                t::STATE => {
                    let ph = phase(d.u8()?)?;
                    let offer = if d.bool()? { Some(Offer { version: d.str()?, build: d.opt_str()?, sha256: d.str()?, size: d.u64()?, url: d.str()?, notes: d.opt_str()? }) } else { None };
                    let fraction = d.f32()?;
                    let mut apps = Vec::new();
                    for _ in 0..d.u32()? {
                        apps.push((d.str()?, d.str()?));
                    }
                    let mut system = Vec::new();
                    for _ in 0..d.u32()? {
                        system.push(d.str()?);
                    }
                    Evt::State(State { phase: ph, offer, fraction, apps, system, error: d.opt_str()? })
                }
                t::SYSTEM_INSTALLED => {
                    let mut ids = Vec::new();
                    for _ in 0..d.u32()? {
                        ids.push(d.str()?);
                    }
                    Evt::SystemInstalled(ids)
                }
                other => return Err(DecodeError::UnknownTag(other)),
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn commands_round_trip() {
            let cmds = vec![
                Cmd::Ping,
                Cmd::Subscribe,
                Cmd::Configure { version: "0.4.0".into(), git: "abc123".into(), auto_download: true, url: None },
                Cmd::Configure { version: "0.4.0".into(), git: String::new(), auto_download: false, url: Some("http://lan/latest.json".into()) },
                Cmd::Conditions { wifi: true, low_power: false, charging: true },
                Cmd::Check,
                Cmd::Download,
            ];
            for c in cmds {
                assert_eq!(Cmd::decode(&c.encode()), Ok(c));
            }
        }

        #[test]
        fn events_round_trip() {
            let evts = vec![
                Evt::State(State { phase: Phase::Idle, offer: None, fraction: 0.0, apps: vec![], system: vec![], error: None }),
                Evt::State(State {
                    phase: Phase::Downloading,
                    offer: Some(Offer { version: "0.5.0".into(), build: Some("g1".into()), sha256: "ff".into(), size: 1 << 33, url: "https://x/springboard".into(), notes: Some("Fixes".into()) }),
                    fraction: 0.25,
                    apps: vec![("is.olibuijr.notes".into(), "1.2.0".into())],
                    system: vec!["is.olibuijr.keyboard".into()],
                    error: Some("offline".into()),
                }),
                Evt::SystemInstalled(vec!["is.olibuijr.keyboard".into()]),
            ];
            for e in evts {
                assert_eq!(Evt::decode(&e.encode()), Ok(e));
            }
        }
    }
}
