//! Now Playing for apps: publish a session to Control Center and receive its transport commands.
//!
//! [`MediaSession`] is the app's session: set its [`MediaContent`] and playback with `set_content` and `set_playback`,
//! declare each [`RemoteCommand`] as `Unsupported`, `Disabled` or `Enabled` with `set_command`. [`publish`] sends it to
//! the shell's `now_playing` service (Control Center's media tile shows the newest session) and [`clear`] withdraws it.
//! When the user taps play, pause, previous, next or seek on the tile, the shell sends a `command` event to the app
//! (`App::service_event`); decode it with [`command_from_event`] and check it against the session before acting.
//!
//! Artwork: [`Artwork`] is an identifier with the image sizes the item can supply. [`Artwork::choose`] picks the size to
//! load for a target, [`aspect_fit`] sizes it inside a box and [`request_pixels`] turns points and the display scale into
//! the pixel size passed to the artwork provider. Artwork does not reach the shell yet.
//!
//! The models are `bifrost_ui::media_session`, `bifrost_ui::media_content` and `bifrost_ui::media_artwork`. The shell side
//! is `springboard/src/shell/services/now_playing.rs`.

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

pub use bifrost_ui::media_artwork::{aspect_fit, request_pixels, Artwork, ArtworkError, ArtworkSize, MAX_SIDE};
pub use bifrost_ui::media_content::{ContentKind, MediaContent, MediaDuration, MediaType};
pub use bifrost_ui::media_session::{
    CommandState, MediaPlaybackSnapshot, MediaSession, MediaSessionError, PlaybackState, RemoteCommand,
};

/// The shell service name.
pub const SERVICE: &str = "now_playing";

/// The event name the shell uses for a transport command.
pub const COMMAND_EVENT: &str = "command";

/// The command names on the wire, one per [`RemoteCommand`].
const NAMES: [(&str, RemoteCommand); 9] = [
    ("play", RemoteCommand::Play),
    ("pause", RemoteCommand::Pause),
    ("stop", RemoteCommand::Stop),
    ("toggle", RemoteCommand::TogglePlayPause),
    ("next", RemoteCommand::Next),
    ("previous", RemoteCommand::Previous),
    ("skip_forward", RemoteCommand::SkipForward),
    ("skip_backward", RemoteCommand::SkipBackward),
    ("seek", RemoteCommand::SeekTo),
];

/// A transport command the shell sent to the app. `Seek` carries the position in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Stop,
    Toggle,
    Next,
    Previous,
    SkipForward,
    SkipBackward,
    Seek(f64),
}

impl Command {
    /// The session command this shell command asks for. A seek is the seek-to-position command.
    pub fn remote(self) -> RemoteCommand {
        match self {
            Command::Play => RemoteCommand::Play,
            Command::Pause => RemoteCommand::Pause,
            Command::Stop => RemoteCommand::Stop,
            Command::Toggle => RemoteCommand::TogglePlayPause,
            Command::Next => RemoteCommand::Next,
            Command::Previous => RemoteCommand::Previous,
            Command::SkipForward => RemoteCommand::SkipForward,
            Command::SkipBackward => RemoteCommand::SkipBackward,
            Command::Seek(_) => RemoteCommand::SeekTo,
        }
    }
}

/// Checks a shell command against the commands the app declared on its session. `Ok` is the session command the app
/// should run. An undeclared command is refused as unsupported, and a declared but disabled one as disabled.
pub fn accept(session: &MediaSession, command: Command) -> Result<RemoteCommand, MediaSessionError> {
    let remote = command.remote();
    session.dispatch(remote)?;
    Ok(remote)
}

/// The typed command in a `command` service event from the shell, or `None` for any other event or name.
pub fn command_from_event(service: &str, event: &str, body: &Json) -> Option<Command> {
    if service != SERVICE || event != COMMAND_EVENT {
        return None;
    }
    let name = body.str_field("command")?;
    Some(match name.as_str() {
        "play" => Command::Play,
        "pause" => Command::Pause,
        "stop" => Command::Stop,
        "toggle" => Command::Toggle,
        "next" => Command::Next,
        "previous" => Command::Previous,
        "skip_forward" => Command::SkipForward,
        "skip_backward" => Command::SkipBackward,
        "seek" => Command::Seek(body.get("position_ms").and_then(Json::number)?),
        _ => return None,
    })
}

/// The `publish` body for a session: the fields the shell's `now_playing` service reads. `None` without content.
pub fn publish_body(session: &MediaSession) -> Option<Json> {
    let content = session.content()?;
    let text = |key: &str, value: &str| (key.to_string(), Json::Str(value.to_string()));
    let mut fields = vec![text("id", session.id()), text("title", &content.title)];
    if let Some(artist) = &content.artist {
        fields.push(text("artist", artist));
    }
    if let Some(album) = &content.album {
        fields.push(text("album", album));
    }
    fields.push(text("type", if content.media_type == MediaType::Video { "video" } else { "audio" }));
    match content.duration {
        MediaDuration::Finite(seconds) => fields.push(("duration_ms".to_string(), Json::Num(seconds * 1000.0))),
        MediaDuration::Live => fields.push(("live".to_string(), Json::Bool(true))),
        MediaDuration::Continuous => {}
    }
    let snapshot = session.snapshot();
    let state = match snapshot.state {
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopped => "stopped",
    };
    fields.push(text("state", state));
    fields.push(("position_ms".to_string(), Json::Num(snapshot.position_ms as f64)));
    fields.push(("rate".to_string(), Json::Num(snapshot.rate() as f64)));
    fields.push(text("kind", content.kind.name()));
    let commands: Vec<(String, Json)> = NAMES
        .iter()
        .filter_map(|(name, command)| match session.command_state(*command) {
            CommandState::Unsupported => None,
            CommandState::Disabled => Some(text(name, "disabled")),
            CommandState::Enabled => Some(text(name, "enabled")),
        })
        .collect();
    fields.push(("commands".to_string(), Json::Obj(commands)));
    Some(Json::Obj(fields))
}

/// Publishes the session to Control Center. Returns the request id, or `None` when the session has no content.
pub fn publish(ctx: &Ctx, session: &MediaSession) -> Option<u32> {
    Some(service::call(ctx, SERVICE, "publish", &publish_body(session)?))
}

/// Withdraws the app's session from Control Center. Returns the request id.
pub fn clear(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "clear", &Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> MediaSession {
        let mut s = MediaSession::new("main").expect("a session");
        let mut content = MediaContent::new("t1", "Song", MediaType::Audio, MediaDuration::Finite(200.0)).expect("content");
        content.artist = Some("Band".into());
        s.set_content(Some(content));
        s.set_playback(PlaybackState::Playing, 5_000);
        s.set_command(RemoteCommand::Play, CommandState::Enabled);
        s.set_command(RemoteCommand::Next, CommandState::Disabled);
        s
    }

    #[test]
    fn a_publish_body_carries_the_content_state_and_declared_commands() {
        let body = publish_body(&session()).expect("a body");
        assert_eq!(body.str_field("title").as_deref(), Some("Song"));
        assert_eq!(body.str_field("artist").as_deref(), Some("Band"));
        assert_eq!(body.str_field("state").as_deref(), Some("playing"));
        assert_eq!(body.get("duration_ms").and_then(Json::number), Some(200_000.0));
        assert_eq!(body.get("position_ms").and_then(Json::number), Some(5_000.0));
        let commands = body.get("commands").expect("commands");
        assert_eq!(commands.str_field("play").as_deref(), Some("enabled"));
        assert_eq!(commands.str_field("next").as_deref(), Some("disabled"));
        assert!(commands.get("pause").is_none(), "an unsupported command is not sent");
    }

    #[test]
    fn a_session_without_content_is_not_published() {
        assert!(publish_body(&MediaSession::new("main").unwrap()).is_none());
    }

    #[test]
    fn live_content_is_sent_as_live_not_as_a_duration() {
        let mut s = MediaSession::new("radio").unwrap();
        s.set_content(MediaContent::new("r", "Rás", MediaType::Audio, MediaDuration::Live).ok());
        let body = publish_body(&s).unwrap();
        assert_eq!(body.get("live"), Some(&Json::Bool(true)));
        assert!(body.get("duration_ms").is_none());
    }

    #[test]
    fn shell_commands_decode_into_typed_commands() {
        let play = bifrost_core::json::parse(r#"{"command":"play"}"#).unwrap();
        assert_eq!(command_from_event(SERVICE, COMMAND_EVENT, &play), Some(Command::Play));
        let seek = bifrost_core::json::parse(r#"{"command":"seek","position_ms":90000}"#).unwrap();
        assert_eq!(command_from_event(SERVICE, COMMAND_EVENT, &seek), Some(Command::Seek(90_000.0)));
        assert_eq!(command_from_event("dock", COMMAND_EVENT, &play), None, "another service");
        assert_eq!(command_from_event(SERVICE, "other", &play), None, "another event");
        let bad = bifrost_core::json::parse(r#"{"command":"fly"}"#).unwrap();
        assert_eq!(command_from_event(SERVICE, COMMAND_EVENT, &bad), None);
    }
}
