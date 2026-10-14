//! Video playback with an app-chosen audio output: a typed facade over the shell's media player.
//!
//! The player writes its sound through an [`AudioOut`]. This module lets an app choose that output for a video (the
//! system speaker, or a silent output that keeps the clock running) and maps the player's open failures to
//! [`VideoError`]. Demuxing, decoding and the A/V clock stay in the player; this module adds no decoder.

use std::fmt;
use std::io;
use std::path::Path;

use springboard::media::aac::OUT_RATE;
use springboard::media::audio_out::{AudioOut, NullOut};
use springboard::media::player::Player;
use springboard::media::speaker::SpeakerOut;

/// Where a video's sound goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoAudio {
    /// The default output device, shared with the system's other audio.
    Speaker,
    /// Discarded: the frames are paced like a real output, but nothing is played.
    Silent,
}

/// Why a video could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// The file does not exist.
    NotFound,
    /// The file exists but could not be opened or started; the text is the system's reason.
    Unreadable(String),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideoError::NotFound => f.write_str("the video file does not exist"),
            VideoError::Unreadable(reason) => write!(f, "the video could not be read: {reason}"),
        }
    }
}

impl std::error::Error for VideoError {}

/// The audio output for a route. The speaker opens its device on the first write, so creating it is cheap.
pub fn audio_output(route: VideoAudio) -> Box<dyn AudioOut> {
    match route {
        VideoAudio::Speaker => Box::new(SpeakerOut::new(OUT_RATE, 2)),
        VideoAudio::Silent => Box::new(NullOut::new()),
    }
}

/// Opens the video at `path` with its sound sent to `audio`, and starts its threads.
pub fn open(path: &Path, audio: VideoAudio) -> Result<Player, VideoError> {
    Player::open_audio(path, audio_output(audio)).map_err(map_open_error)
}

fn map_open_error(err: io::Error) -> VideoError {
    match err.kind() {
        io::ErrorKind::NotFound => VideoError::NotFound,
        _ => VideoError::Unreadable(err.to_string()),
    }
}
