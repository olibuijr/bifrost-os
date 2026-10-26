//! Now Playing for apps: publish a session to Control Center and receive its transport commands.
//!
//! [`MediaSession`] is the app's session: set its [`MediaContent`] and playback with `set_content` and `set_playback`,
//! declare each [`RemoteCommand`] as `Unsupported`, `Disabled` or `Enabled` with `set_command`. [`publish`] sends it to
//! the shell's `now_playing` service (Control Center's media tile shows the newest session) and [`clear`] withdraws it.
//! When the user taps play, pause, previous, next or seek on the tile, the shell sends a `command` event to the app
//! (`App::service_event`); decode it with [`command_from_event`] and check it against the session before acting.
//!
//! Up Next: [`publish_with_queue`] also sends the app's queue of [`QueueEntry`] values. When the user picks an entry
//! in Control Center, the shell sends a `queue_select` event; decode it with [`queue_pick`] to get the entry's index.
//!
//! Artwork: [`Artwork`] is an identifier with the image sizes the item can supply. [`Artwork::choose`] picks the size to
//! load for a target, [`aspect_fit`] sizes it inside a box and [`request_pixels`] turns points and the display scale into
//! the pixel size passed to the artwork provider. The pixels themselves are an [`ArtworkImage`] (straight RGBA, at most
//! [`WIRE_SIDE_MAX`] on a side): [`publish_with_artwork`] or [`with_artwork`] sends them as the publish body's `artwork`
//! field, and the media tile, the Now Playing detail, the menu bar extra and the lock card draw them in place of the
//! glyph. A session without artwork, or with an `artwork` the shell cannot read, keeps the glyph.
//!
//! The models are `bifrost_ui::media_session`, `bifrost_ui::media_content` and `bifrost_ui::media_artwork`. The shell side
//! is `springboard/src/shell/services/now_playing.rs`.

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

pub use bifrost_ui::media_artwork::{aspect_fit, request_pixels, Artwork, ArtworkError, ArtworkImage, ArtworkSize, MAX_SIDE, WIRE_SIDE_MAX};
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

/// The metadata an accessory display shows for a session: the publish body without the transport command map and the
/// content kind, which a display cannot act on. None without content. No accessory transport is connected yet, so the
/// shell does not send this body anywhere.
pub fn accessory_body(session: &MediaSession) -> Option<Json> {
    let Json::Obj(fields) = publish_body(session)? else { return None };
    let shown = fields.into_iter().filter(|(key, _)| key != "commands" && key != "kind").collect();
    Some(Json::Obj(shown))
}

/// One entry of an app's Up Next queue. An entry the app cannot play now is listed by the shell but cannot be picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub title: String,
    pub artist: Option<String>,
    pub playable: bool,
}

impl QueueEntry {
    /// A playable entry with a title and no artist.
    pub fn new(title: impl Into<String>) -> QueueEntry {
        QueueEntry { title: title.into(), artist: None, playable: true }
    }
}

/// The `queue` field of a publish body: one object per entry, in queue order.
pub fn queue_json(queue: &[QueueEntry]) -> Json {
    Json::Arr(
        queue
            .iter()
            .map(|entry| {
                let mut fields = vec![("title".to_string(), Json::Str(entry.title.clone()))];
                if let Some(artist) = &entry.artist {
                    fields.push(("artist".to_string(), Json::Str(artist.clone())));
                }
                fields.push(("playable".to_string(), Json::Bool(entry.playable)));
                Json::Obj(fields)
            })
            .collect(),
    )
}

/// The `publish` body for a session with its Up Next queue. `None` without content.
pub fn publish_queue_body(session: &MediaSession, queue: &[QueueEntry]) -> Option<Json> {
    let mut body = publish_body(session)?;
    if let Json::Obj(fields) = &mut body {
        fields.push(("queue".to_string(), queue_json(queue)));
    }
    Some(body)
}

/// Publishes the session to Control Center. Returns the request id, or `None` when the session has no content.
pub fn publish(ctx: &Ctx, session: &MediaSession) -> Option<u32> {
    Some(service::call(ctx, SERVICE, "publish", &publish_body(session)?))
}

/// Publishes the session with its Up Next queue. Returns the request id, or `None` when the session has no content.
pub fn publish_with_queue(ctx: &Ctx, session: &MediaSession, queue: &[QueueEntry]) -> Option<u32> {
    Some(service::call(ctx, SERVICE, "publish", &publish_queue_body(session, queue)?))
}

/// The index of the Up Next entry the user picked, from a `queue_select` event. None for any other event.
pub fn queue_pick(service: &str, event: &str, body: &Json) -> Option<usize> {
    if service != SERVICE || event != COMMAND_EVENT || body.str_field("command").as_deref() != Some("queue_select") {
        return None;
    }
    let index = body.get("index").and_then(Json::number)?;
    (index.is_finite() && index >= 0.0 && index.fract() == 0.0).then_some(index as usize)
}

/// The publish body with the session's artwork: `body` plus an `artwork` field (`{w, h, rgba}`, see
/// [`ArtworkImage::to_json`]). `None` leaves the body as it is, so the shell shows its glyph. A body that is not an
/// object is returned unchanged.
pub fn with_artwork(mut body: Json, artwork: Option<&ArtworkImage>) -> Json {
    if let (Json::Obj(fields), Some(art)) = (&mut body, artwork) {
        fields.retain(|(key, _)| key != "artwork");
        fields.push(("artwork".to_string(), art.to_json()));
    }
    body
}

/// Publishes the session with its Up Next queue and its artwork. Returns the request id, or `None` when the session
/// has no content.
pub fn publish_with_artwork(ctx: &Ctx, session: &MediaSession, queue: &[QueueEntry], artwork: Option<&ArtworkImage>) -> Option<u32> {
    let body = with_artwork(publish_queue_body(session, queue)?, artwork);
    Some(service::call(ctx, SERVICE, "publish", &body))
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
    fn an_accessory_body_shows_the_metadata_and_state_without_commands() {
        let body = accessory_body(&session()).expect("a body");
        assert_eq!(body.str_field("title").as_deref(), Some("Song"));
        assert_eq!(body.str_field("state").as_deref(), Some("playing"));
        assert_eq!(body.get("position_ms").and_then(Json::number), Some(5_000.0));
        assert!(body.get("commands").is_none(), "a display cannot act on the command map");
        assert!(body.get("kind").is_none());
        assert!(accessory_body(&MediaSession::new("main").unwrap()).is_none(), "no content, nothing to show");
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
    fn a_queue_publishes_with_the_session_and_a_pick_decodes_to_its_index() {
        let queue = [QueueEntry { title: "Next".into(), artist: Some("Band".into()), playable: true }, QueueEntry { title: "Gone".into(), artist: None, playable: false }];
        let body = publish_queue_body(&session(), &queue).expect("a body");
        let entries = body.get("queue").and_then(Json::as_array).expect("a queue");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].str_field("artist").as_deref(), Some("Band"));
        assert!(entries[1].str_field("artist").is_none());
        assert_eq!(entries[1].get("playable"), Some(&Json::Bool(false)));
        assert!(publish_queue_body(&MediaSession::new("main").unwrap(), &queue).is_none(), "no content, no publish");
        let pick = bifrost_core::json::parse(r#"{"command":"queue_select","index":1}"#).unwrap();
        assert_eq!(queue_pick(SERVICE, COMMAND_EVENT, &pick), Some(1));
        assert_eq!(queue_pick("dock", COMMAND_EVENT, &pick), None, "another service");
        let play = bifrost_core::json::parse(r#"{"command":"play"}"#).unwrap();
        assert_eq!(queue_pick(SERVICE, COMMAND_EVENT, &play), None, "another command");
        let negative = bifrost_core::json::parse(r#"{"command":"queue_select","index":-1}"#).unwrap();
        assert_eq!(queue_pick(SERVICE, COMMAND_EVENT, &negative), None, "not an index");
    }

    #[test]
    fn artwork_rides_in_the_publish_body_and_is_optional() {
        let art = ArtworkImage::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).expect("pixels");
        let body = with_artwork(publish_body(&session()).expect("a body"), Some(&art));
        let sent = bifrost_core::json::parse(&body.encode()).expect("valid json");
        assert_eq!(ArtworkImage::from_json(sent.get("artwork").expect("an artwork field")), Ok(art.clone()));
        assert_eq!(sent.str_field("title").as_deref(), Some("Song"), "the rest of the body is untouched");
        let plain = with_artwork(publish_body(&session()).expect("a body"), None);
        assert!(plain.get("artwork").is_none(), "no artwork, no field: the shell keeps its glyph");
        let again = with_artwork(body, Some(&art));
        let Json::Obj(fields) = &again else { panic!("an object") };
        assert_eq!(fields.iter().filter(|(k, _)| k == "artwork").count(), 1, "a second call replaces the first");
        assert_eq!(with_artwork(Json::Null, Some(&art)), Json::Null);
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
