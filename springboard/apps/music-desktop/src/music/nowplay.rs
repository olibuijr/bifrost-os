//! Now Playing for the Music app. The app keeps one local `MediaSession` (the model in `bifrost_ui::media_session`,
//! reached through `bifrost_app::now_playing`) in step with the audio service's snapshot: the song or station as
//! content, the playback state and position, and which transport commands are enabled. A system command is checked
//! against the session first and then runs the app's own transport (`Music::control`).
//!
//! The session is not published to the lock card or Control Center: the audio service carries no app session, so the
//! shell still shows only what the service plays. Delivery of system commands needs the shell's `now-playing` service
//! events to reach the app (see `docs/parity/wire-app-music.md`).

use crate::music::engine::{Repeat, Snapshot, Track};
use bifrost_app::now_playing::{CommandState, MediaContent, MediaDuration, MediaSession, MediaType, PlaybackState, RemoteCommand};
use springboard::shell::music::nowplaying::Target;

/// The session identifier: the app's bundle id.
pub const SESSION_ID: &str = "is.olibuijr.music.desktop";

/// The app's Now Playing session, or None when the identifier is refused (it never is).
pub fn session() -> Option<MediaSession> {
    MediaSession::new(SESSION_ID).ok()
}

fn text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The content of the playing item: a song with its length, or a station as a live item. None when the title is
/// empty. A song without a usable length is continuous content.
pub fn content(track: &Track, title: &str, artist: &str, album: &str, duration_ms: Option<f64>) -> Option<MediaContent> {
    let (id, duration) = if track.is_live() {
        (format!("radio:{}", track.id), MediaDuration::Live)
    } else {
        let d = duration_ms.and_then(|ms| MediaDuration::finite_seconds(ms / 1000.0).ok()).unwrap_or(MediaDuration::Continuous);
        (format!("song:{}", track.id), d)
    };
    let mut c = MediaContent::new(&id, title, MediaType::Audio, duration).ok()?;
    c.artist = text(artist);
    c.album = text(album);
    Some(c)
}

/// The published playback state: stopped with no track, else playing or paused.
pub fn playback(snap: &Snapshot) -> PlaybackState {
    match (&snap.track, snap.playing) {
        (None, _) => PlaybackState::Stopped,
        (Some(_), true) => PlaybackState::Playing,
        (Some(_), false) => PlaybackState::Paused,
    }
}

/// Declares which commands the app can run now. Commands the app has no action for stay unsupported (the session's
/// default). Disabled means the action exists but cannot run at this moment.
pub fn declare(session: &mut MediaSession, snap: &Snapshot) {
    let has = snap.track.is_some();
    let seekable = has && snap.track.as_ref().is_some_and(|t| !t.is_live()) && snap.duration_ms.is_some();
    let next = snap.index.is_some_and(|i| i + 1 < snap.queue_len) || (has && snap.repeat == Repeat::All);
    let state = |on: bool| if on { CommandState::Enabled } else { CommandState::Disabled };
    session.set_command(RemoteCommand::Play, state(has && !snap.playing));
    session.set_command(RemoteCommand::Pause, state(has && snap.playing));
    session.set_command(RemoteCommand::TogglePlayPause, state(has));
    session.set_command(RemoteCommand::Next, state(next));
    session.set_command(RemoteCommand::Previous, state(has));
    session.set_command(RemoteCommand::SeekTo, state(seekable));
}

/// Publishes the content, playback and command table for the snapshot. The content is set first: the session clamps
/// the position to the content's length.
pub fn sync(session: &mut MediaSession, snap: &Snapshot, content: Option<MediaContent>) {
    session.set_content(content);
    session.set_playback(playback(snap), snap.position_ms.max(0.0) as u64);
    declare(session, snap);
}

/// The command a system name stands for (`play`, `pause`, `stop`, `toggle`, `next`, `previous`).
pub fn command_named(name: &str) -> Option<RemoteCommand> {
    match name {
        "play" => Some(RemoteCommand::Play),
        "pause" => Some(RemoteCommand::Pause),
        "stop" => Some(RemoteCommand::Stop),
        "toggle" | "togglePlayPause" => Some(RemoteCommand::TogglePlayPause),
        "next" => Some(RemoteCommand::Next),
        "previous" | "prev" => Some(RemoteCommand::Previous),
        _ => None,
    }
}

/// The transport action for a command, given whether the player is playing now. None when the app has no action for
/// it (Stop, skips), or when the action would do the opposite of the request (Pause while paused).
pub fn target(cmd: RemoteCommand, playing: bool) -> Option<Target> {
    match cmd {
        RemoteCommand::Play => (!playing).then_some(Target::Play),
        RemoteCommand::Pause => playing.then_some(Target::Play),
        RemoteCommand::TogglePlayPause => Some(Target::Play),
        RemoteCommand::Next => Some(Target::Next),
        RemoteCommand::Previous => Some(Target::Prev),
        _ => None,
    }
}

/// Checks a command on the session and returns the transport action to run. Nothing runs when the session has not
/// enabled the command.
pub fn route(session: &MediaSession, cmd: RemoteCommand, playing: bool) -> Option<Target> {
    session.dispatch(cmd).ok()?;
    target(cmd, playing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn snap(track: Option<Track>, index: Option<usize>, playing: bool, queue_len: usize, repeat: Repeat) -> Snapshot {
        Snapshot {
            track,
            index,
            playing,
            position_ms: 12_345.6,
            duration_ms: Some(180_000.0),
            shuffle: false,
            repeat,
            queue_len,
            volume: 1.0,
            error: None,
            live: None,
        }
    }

    fn song() -> Track {
        Track::file(7, PathBuf::from("/music/a.mp3"))
    }

    #[test]
    fn session_has_the_app_identifier() {
        assert_eq!(session().map(|s| s.id().to_string()), Some(SESSION_ID.to_string()));
    }

    #[test]
    fn content_of_a_song_has_a_finite_length_and_trimmed_text() {
        let c = content(&song(), "  Song  ", " Artist ", "", Some(180_000.0)).unwrap();
        assert_eq!(c.title, "Song");
        assert_eq!(c.artist.as_deref(), Some("Artist"));
        assert_eq!(c.album, None);
        assert_eq!(c.duration, MediaDuration::Finite(180.0));
        assert_eq!(c.media_type, MediaType::Audio);
    }

    #[test]
    fn content_without_a_usable_length_is_continuous_and_empty_titles_are_refused() {
        assert_eq!(content(&song(), "T", "", "", Some(0.0)).unwrap().duration, MediaDuration::Continuous);
        assert_eq!(content(&song(), "T", "", "", None).unwrap().duration, MediaDuration::Continuous);
        assert!(content(&song(), "   ", "", "", Some(1.0)).is_none());
    }

    #[test]
    fn a_station_is_live_content() {
        let station = Track::live(3, "https://example.invalid/stream".into());
        let c = content(&station, "Rás 2", "On air", "", Some(5.0)).unwrap();
        assert_eq!(c.duration, MediaDuration::Live);
        assert_eq!(c.id, "radio:3");
    }

    #[test]
    fn sync_publishes_content_playback_and_position() {
        let mut s = session().unwrap();
        let sn = snap(Some(song()), Some(0), true, 2, Repeat::Off);
        sync(&mut s, &sn, content(&song(), "Song", "Artist", "Album", sn.duration_ms));
        assert_eq!(s.content().map(|c| c.title.as_str()), Some("Song"));
        let published = s.snapshot();
        assert_eq!(published.state, PlaybackState::Playing);
        assert_eq!(published.position_ms, 12_345);
    }

    #[test]
    fn no_track_is_stopped_and_only_the_plain_toggle_is_offered() {
        let mut s = session().unwrap();
        sync(&mut s, &snap(None, None, false, 0, Repeat::Off), None);
        assert_eq!(s.snapshot().state, PlaybackState::Stopped);
        assert_eq!(s.command_state(RemoteCommand::Play), CommandState::Disabled);
        assert_eq!(s.command_state(RemoteCommand::Next), CommandState::Disabled);
        assert_eq!(s.command_state(RemoteCommand::SeekTo), CommandState::Disabled);
        assert_eq!(s.command_state(RemoteCommand::Stop), CommandState::Unsupported);
    }

    #[test]
    fn next_is_enabled_only_with_a_following_item_or_repeat_all() {
        let mut s = session().unwrap();
        declare(&mut s, &snap(Some(song()), Some(0), true, 2, Repeat::Off));
        assert_eq!(s.command_state(RemoteCommand::Next), CommandState::Enabled);
        declare(&mut s, &snap(Some(song()), Some(1), true, 2, Repeat::Off));
        assert_eq!(s.command_state(RemoteCommand::Next), CommandState::Disabled);
        declare(&mut s, &snap(Some(song()), Some(1), true, 2, Repeat::All));
        assert_eq!(s.command_state(RemoteCommand::Next), CommandState::Enabled);
    }

    #[test]
    fn play_and_pause_are_offered_by_state() {
        let mut s = session().unwrap();
        declare(&mut s, &snap(Some(song()), Some(0), true, 1, Repeat::Off));
        assert_eq!(s.command_state(RemoteCommand::Pause), CommandState::Enabled);
        assert_eq!(s.command_state(RemoteCommand::Play), CommandState::Disabled);
        assert_eq!(s.command_state(RemoteCommand::SeekTo), CommandState::Enabled);
    }

    #[test]
    fn a_system_command_runs_only_when_the_session_enables_it() {
        let mut s = session().unwrap();
        declare(&mut s, &snap(Some(song()), Some(0), true, 1, Repeat::Off));
        assert!(matches!(route(&s, RemoteCommand::Pause, true), Some(Target::Play)));
        assert!(route(&s, RemoteCommand::Play, true).is_none());
        assert!(route(&s, RemoteCommand::Next, true).is_none());
        assert!(route(&s, RemoteCommand::Stop, true).is_none());
    }

    #[test]
    fn names_map_to_commands_and_actions() {
        assert_eq!(command_named("toggle"), Some(RemoteCommand::TogglePlayPause));
        assert_eq!(command_named("prev"), Some(RemoteCommand::Previous));
        assert_eq!(command_named("seek"), None);
        assert!(matches!(target(RemoteCommand::Play, false), Some(Target::Play)));
        assert!(target(RemoteCommand::Play, true).is_none());
        assert!(matches!(target(RemoteCommand::Previous, true), Some(Target::Prev)));
        assert!(matches!(target(RemoteCommand::Next, false), Some(Target::Next)));
    }
}
