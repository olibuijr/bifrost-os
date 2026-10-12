//! Music's glue to the shell's system services, as rules that need no window: which transport command a Control Center
//! tap means, when the audio route has lost its private output (headphones or Bluetooth), when a Now Playing publish is
//! due, and what counts as a key change. The calls themselves are made in `app.rs`.
//!
//! Services used: `now_playing` (Control Center's media tile), `audio-devices` (route changes) and `dock` (progress).

use bifrost_app::now_playing::Command;
use bifrost_core::json::Json;

/// A position this far from where the last publish put it (after the time that passed while playing) is a seek.
pub const SEEK_MS: f64 = 2_000.0;

/// The name `Music::remote_command` takes for a transport command, or None for commands with no transport action
/// here: Stop and the skips are not offered by the session, and a seek takes a position (`Music::remote_seek`).
pub fn transport_name(cmd: &Command) -> Option<&'static str> {
    match cmd {
        Command::Play => Some("play"),
        Command::Pause => Some("pause"),
        Command::Toggle => Some("toggle"),
        Command::Next => Some("next"),
        Command::Previous => Some("previous"),
        Command::Stop | Command::SkipForward | Command::SkipBackward | Command::Seek(_) => None,
    }
}

/// True when the output moved off headphones or Bluetooth onto the speaker, or lost its device. Music pauses then, as
/// it does when headphones come out. A switch between two private outputs (a new Bluetooth device) does not pause.
pub fn output_lost(before: Option<&str>, after: Option<&str>) -> bool {
    let private = |o: Option<&str>| matches!(o, Some("headphones" | "bluetooth"));
    private(before) && !private(after)
}

/// True when the playback position is not where the last publish says it should be: more than [`SEEK_MS`] away from
/// `published_ms` plus the time since `published_at` while it was playing. Normal playback stays within that margin, so
/// only a seek makes it true. Times are in milliseconds on the app's clock.
pub fn seeked(published_ms: f64, published_at: f64, now: f64, playing: bool, position_ms: f64) -> bool {
    let expected = published_ms + if playing { now - published_at } else { 0.0 };
    (position_ms - expected).abs() > SEEK_MS
}

/// The publish body without its position. A change in this key is one the tile must show at once (a new song, the
/// state, the commands); the position alone is sent on a seek (see [`seeked`]).
pub fn publish_key(body: &Json) -> String {
    match body {
        Json::Obj(fields) => Json::Obj(fields.iter().filter(|(k, _)| k.as_str() != "position_ms").cloned().collect()).encode(),
        other => other.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(title: &str, state: &str, position: f64) -> Json {
        Json::Obj(vec![
            ("title".into(), Json::Str(title.into())),
            ("state".into(), Json::Str(state.into())),
            ("position_ms".into(), Json::Num(position)),
        ])
    }

    #[test]
    fn transport_commands_map_to_the_names_the_transport_takes() {
        assert_eq!(transport_name(&Command::Play), Some("play"));
        assert_eq!(transport_name(&Command::Pause), Some("pause"));
        assert_eq!(transport_name(&Command::Toggle), Some("toggle"));
        assert_eq!(transport_name(&Command::Next), Some("next"));
        assert_eq!(transport_name(&Command::Previous), Some("previous"));
    }

    #[test]
    fn stop_skips_and_seek_have_no_transport_name() {
        assert_eq!(transport_name(&Command::Stop), None);
        assert_eq!(transport_name(&Command::SkipForward), None);
        assert_eq!(transport_name(&Command::SkipBackward), None);
        assert_eq!(transport_name(&Command::Seek(1_000.0)), None);
    }

    #[test]
    fn losing_headphones_or_bluetooth_pauses_but_switching_between_them_does_not() {
        assert!(output_lost(Some("headphones"), Some("speaker")));
        assert!(output_lost(Some("bluetooth"), Some("speaker")));
        assert!(output_lost(Some("headphones"), None));
        assert!(!output_lost(Some("bluetooth"), Some("headphones")));
        assert!(!output_lost(Some("speaker"), Some("speaker")));
        assert!(!output_lost(None, Some("speaker")));
        assert!(!output_lost(Some("speaker"), Some("headphones")));
    }

    #[test]
    fn normal_playback_is_not_a_seek_but_a_jump_is() {
        // published at 1 s playing; at 2.5 s the position is 2.5 s: expected 1 s + 1.5 s, no seek
        assert!(!seeked(1_000.0, 0.0, 1_500.0, true, 2_500.0));
        // a scrub to 60 s
        assert!(seeked(1_000.0, 0.0, 1_500.0, true, 60_000.0));
    }

    #[test]
    fn a_paused_position_does_not_drift_into_a_seek() {
        assert!(!seeked(1_000.0, 0.0, 30_000.0, false, 1_000.0));
        assert!(seeked(1_000.0, 0.0, 30_000.0, false, 9_000.0));
    }

    #[test]
    fn the_key_ignores_the_position_and_follows_everything_else() {
        assert_eq!(publish_key(&publish("A", "playing", 1.0)), publish_key(&publish("A", "playing", 99_000.0)));
        assert_ne!(publish_key(&publish("A", "playing", 1.0)), publish_key(&publish("A", "paused", 1.0)));
        assert_ne!(publish_key(&publish("A", "playing", 1.0)), publish_key(&publish("B", "playing", 1.0)));
    }
}
