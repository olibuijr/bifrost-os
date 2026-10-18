//! Typed client for the system audio service (`is.olibuijr.audio`, run by `audiod`): playback, the queue, volume, mute,
//! and microphone recording over the service socket.
//!
//! Rules of this model:
//! - A [`Command`] can only be built through its constructors, and each one checks its value first: a volume is 0 to 1,
//!   a seek is a finite position of at least zero, a queue has at least one source and a start inside it, and a
//!   recording goes to an absolute `.wav` path.
//! - A file [`Source`] is an absolute path. A stream source is an `http://` or `https://` URL with no whitespace.
//! - The socket is `BIFROST_SERVICE_SOCK` when that is set and not empty, otherwise [`DEFAULT_SOCKET`].
//! - Frames use the service wire format of `bifrost_proto::svc`. Commands get no reply. `ping` and `subscribe` read one.
//! - The client does not start the service, which the supervisor starts on the first connection. No app uses it yet.

use bifrost_proto::svc::{self, audio::Cmd, audio::Track};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

pub use bifrost_proto::svc::audio::{Evt, Repeat, Snapshot};

/// The service's id on the service socket.
pub const SERVICE_ID: &str = "is.olibuijr.audio";
/// The environment variable that overrides the socket path.
pub const SOCKET_ENV: &str = "BIFROST_SERVICE_SOCK";
/// The socket the service listens on when no override is set.
pub const DEFAULT_SOCKET: &str = "/var/lib/springboard/run/is.olibuijr.audio.sock";
/// The slowest and fastest playback rates the service takes.
pub const RATE_MIN: f32 = 0.5;
pub const RATE_MAX: f32 = 3.0;

/// Why a command is refused, or a call to the service fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    EmptyQueue,
    StartOutOfRange { start: u32, len: usize },
    InvalidVolume,
    /// A playback rate outside 0.5 to 3.0, or not a number.
    InvalidRate,
    InvalidPosition,
    EmptyPath,
    RelativePath,
    InvalidStreamUrl,
    NotWavPath,
    Io(std::io::ErrorKind),
    /// A reply frame does not decode.
    Protocol,
    /// A reply is not the one the call waits for.
    UnexpectedReply,
}

/// The socket path for an optional override value. An empty override counts as unset.
pub fn socket_path(override_path: Option<&str>) -> PathBuf {
    match override_path {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(DEFAULT_SOCKET),
    }
}

/// Connects to the service at the socket that [`socket_path`] picks from the environment.
pub fn connect_default() -> Result<AudioClient<UnixStream>, AudioError> {
    let override_path = std::env::var(SOCKET_ENV).ok();
    let stream = UnixStream::connect(socket_path(override_path.as_deref())).map_err(|e| AudioError::Io(e.kind()))?;
    Ok(AudioClient::new(stream))
}

/// One item of a queue: a file on disk or a live stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Stream(String),
}

impl Source {
    pub fn file(path: &str) -> Result<Source, AudioError> {
        check_absolute(path)?;
        Ok(Source::File(PathBuf::from(path)))
    }

    pub fn stream(url: &str) -> Result<Source, AudioError> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).ok_or(AudioError::InvalidStreamUrl)?;
        if rest.is_empty() || url.chars().any(char::is_whitespace) {
            return Err(AudioError::InvalidStreamUrl);
        }
        Ok(Source::Stream(url.to_string()))
    }
}

fn check_absolute(path: &str) -> Result<(), AudioError> {
    if path.is_empty() {
        return Err(AudioError::EmptyPath);
    }
    if !Path::new(path).is_absolute() {
        return Err(AudioError::RelativePath);
    }
    Ok(())
}

/// A command that has passed its checks. The wire form is private, so only the constructors below make one.
#[derive(Clone, Debug, PartialEq)]
pub struct Command(Cmd);

impl Command {
    pub fn play() -> Command {
        Command(Cmd::Play)
    }

    pub fn pause() -> Command {
        Command(Cmd::Pause)
    }

    pub fn toggle() -> Command {
        Command(Cmd::Toggle)
    }

    pub fn next() -> Command {
        Command(Cmd::Next)
    }

    pub fn previous() -> Command {
        Command(Cmd::Prev)
    }

    pub fn stop() -> Command {
        Command(Cmd::Stop)
    }

    pub fn jump_to(index: u32) -> Command {
        Command(Cmd::JumpTo(index))
    }

    pub fn seek(position_ms: f64) -> Result<Command, AudioError> {
        if !position_ms.is_finite() || position_ms < 0.0 {
            return Err(AudioError::InvalidPosition);
        }
        Ok(Command(Cmd::SeekMs(position_ms)))
    }

    pub fn volume(volume: f32) -> Result<Command, AudioError> {
        if !(0.0..=1.0).contains(&volume) {
            return Err(AudioError::InvalidVolume);
        }
        Ok(Command(Cmd::SetVolume(volume)))
    }

    /// Playback speed: 0.5 to 3.0 (1 is normal). audiod keeps the pitch, so only the tempo changes.
    pub fn rate(rate: f32) -> Result<Command, AudioError> {
        if !(RATE_MIN..=RATE_MAX).contains(&rate) {
            return Err(AudioError::InvalidRate);
        }
        Ok(Command(Cmd::SetRate(rate)))
    }

    pub fn mute(muted: bool) -> Command {
        Command(Cmd::SetMuted(muted))
    }

    pub fn repeat(mode: Repeat) -> Command {
        Command(Cmd::SetRepeat(mode))
    }

    pub fn shuffle(on: bool, seed: u64) -> Command {
        Command(Cmd::SetShuffle { on, seed })
    }

    /// Replaces the queue with `sources` and starts at index `start`. Track ids are the source positions plus one.
    pub fn play_queue(sources: &[Source], start: u32, shuffle: bool, seed: u64) -> Result<Command, AudioError> {
        if sources.is_empty() {
            return Err(AudioError::EmptyQueue);
        }
        if start as usize >= sources.len() {
            return Err(AudioError::StartOutOfRange { start, len: sources.len() });
        }
        let tracks = sources.iter().enumerate().map(|(index, source)| track(index as u64 + 1, source)).collect();
        Ok(Command(Cmd::PlayQueue { tracks, start, shuffle, seed }))
    }

    pub fn record_start(path: &str) -> Result<Command, AudioError> {
        check_absolute(path)?;
        if !path.ends_with(".wav") {
            return Err(AudioError::NotWavPath);
        }
        Ok(Command(Cmd::RecordStart { path: path.to_string() }))
    }

    pub fn record_stop() -> Command {
        Command(Cmd::RecordStop)
    }
}

fn track(id: u64, source: &Source) -> Track {
    match source {
        Source::File(path) => Track { id, path: path.to_string_lossy().into_owned(), url: None },
        Source::Stream(url) => Track { id, path: String::new(), url: Some(url.clone()) },
    }
}

/// A client over any byte stream connected to the service. Tests pass one end of a socket pair.
pub struct AudioClient<S> {
    stream: S,
}

impl<S: Read + Write> AudioClient<S> {
    pub fn new(stream: S) -> AudioClient<S> {
        AudioClient { stream }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Sends a command without waiting for a reply.
    pub fn send(&mut self, command: &Command) -> Result<(), AudioError> {
        self.write(&command.0)
    }

    /// Asks the service for its id and version.
    pub fn ping(&mut self) -> Result<(String, String), AudioError> {
        self.write(&Cmd::Ping)?;
        match self.next_event()? {
            Evt::Pong { id, version } => Ok((id, version)),
            _ => Err(AudioError::UnexpectedReply),
        }
    }

    /// Subscribes to state pushes and returns the state at the moment of the subscription.
    pub fn subscribe(&mut self) -> Result<Snapshot, AudioError> {
        self.write(&Cmd::Subscribe)?;
        match self.next_event()? {
            Evt::State(snapshot) => Ok(snapshot),
            _ => Err(AudioError::UnexpectedReply),
        }
    }

    /// Reads the next event the service sent, such as a state push or a recording result.
    pub fn next_event(&mut self) -> Result<Evt, AudioError> {
        let frame = svc::read_frame(&mut self.stream).map_err(|e| AudioError::Io(e.kind()))?;
        Evt::decode(&frame).map_err(|_| AudioError::Protocol)
    }

    fn write(&mut self, cmd: &Cmd) -> Result<(), AudioError> {
        svc::write_frame(&mut self.stream, &cmd.encode()).map_err(|e| AudioError::Io(e.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_command_takes_0_5_to_2_and_nothing_else() {
        assert_eq!(Command::rate(0.5).map(|c| c.0), Ok(Cmd::SetRate(0.5)));
        assert_eq!(Command::rate(1.5).map(|c| c.0), Ok(Cmd::SetRate(1.5)));
        assert_eq!(Command::rate(2.0).map(|c| c.0), Ok(Cmd::SetRate(2.0)));
        assert_eq!(Command::rate(3.0).map(|c| c.0), Ok(Cmd::SetRate(3.0)));
        for bad in [0.49f32, 3.01, 0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(Command::rate(bad), Err(AudioError::InvalidRate), "{bad}");
        }
    }

    #[test]
    fn a_rate_command_is_written_as_a_frame() {
        // the client needs a stream it can read and write: a cursor over an empty buffer
        let mut client = AudioClient::new(std::io::Cursor::new(Vec::<u8>::new()));
        client.send(&Command::rate(1.5).unwrap()).unwrap();
        let bytes = client.into_inner().into_inner();
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(Cmd::decode(&bytes[4..4 + len]), Ok(Cmd::SetRate(1.5)));
    }
}
