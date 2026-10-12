//! The `now_playing` service: an app publishes its Now Playing session (metadata, playback state and the commands it
//! handles) and Control Center's media tile shows the newest one. The tile's transport taps go back to the owner as
//! `command` events, and a seek goes back as a `seek` command with the position in milliseconds.
//!
//! Calls (JSON params; the SDK builds them in `bifrost_app::now_playing`):
//! - `publish` `{id?, title, artist?, album?, type?: "audio"|"video", live?: bool, duration_ms?: number,
//!    state: "playing"|"paused"|"stopped", position_ms?: number, commands?: {name: "enabled"|"disabled"}}`
//!   replaces the shell's session with the caller's. The model is `bifrost_ui::media_session`.
//! - `clear` removes the caller's session when it is the one shown.
//!
//! Events to the owner: `command` with `{"command": "play"|"pause"|"stop"|"toggle"|"next"|"previous"|
//! "skip_forward"|"skip_backward"|"seek"}`, plus `"position_ms"` for `seek`. A command the owner did not enable
//! (`Unsupported` or `Disabled`) is never sent. A session whose app has closed is dropped on the next tick.
//!
//! The session lives in one process-wide store: the shell draws Control Center from it, and the service is answered
//! from the app's channel. Nothing here blocks.
use super::{Event, Shell, SystemService};
use crate::shell::music::nowplaying::Target as Media;
use bifrost_core::json::Json;
use bifrost_ui::media_content::{optional_text, MediaContent, MediaDuration, MediaType};
use bifrost_ui::media_session::{CommandState, MediaSession, PlaybackState, RemoteCommand};
use std::sync::Mutex;

/// The command names an app declares and receives, one per `RemoteCommand`.
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

fn command_name(command: RemoteCommand) -> &'static str {
    NAMES.iter().find(|(_, c)| *c == command).map_or("", |(n, _)| n)
}

/// Reads a `publish` body into the session the caller describes. Errors name the bad field.
pub fn parse_session(app_id: &str, body: &Json) -> Result<MediaSession, String> {
    let id = body.str_field("id").unwrap_or_else(|| app_id.to_string());
    let mut session = MediaSession::new(&id).map_err(|e| format!("bad id: {e:?}"))?;
    let title = body.str_field("title").ok_or_else(|| "publish needs a title".to_string())?;
    let duration = if matches!(body.get("live"), Some(Json::Bool(true))) {
        MediaDuration::Live
    } else {
        match body.get("duration_ms").and_then(Json::number).filter(|ms| ms.is_finite() && *ms > 0.0) {
            Some(ms) => MediaDuration::Finite(ms / 1000.0),
            None => MediaDuration::Continuous,
        }
    };
    let media_type = match body.str_field("type").as_deref() {
        Some("video") => MediaType::Video,
        _ => MediaType::Audio,
    };
    let mut content = MediaContent::new(&id, &title, media_type, duration).map_err(|e| format!("bad content: {e:?}"))?;
    content.artist = body.str_field("artist").and_then(|t| optional_text(&t));
    content.album = body.str_field("album").and_then(|t| optional_text(&t));
    session.set_content(Some(content));
    let state = match body.str_field("state").as_deref() {
        Some("playing") => PlaybackState::Playing,
        Some("paused") => PlaybackState::Paused,
        Some("stopped") | None => PlaybackState::Stopped,
        Some(other) => return Err(format!("unknown state {other}")),
    };
    let position = body.get("position_ms").and_then(Json::number).filter(|p| p.is_finite()).unwrap_or(0.0);
    session.set_playback(state, position.max(0.0) as u64);
    if let Some(Json::Obj(fields)) = body.get("commands") {
        for (name, value) in fields {
            let Some((_, command)) = NAMES.iter().find(|(n, _)| *n == name.as_str()) else {
                return Err(format!("unknown command {name}"));
            };
            let state = match value.as_str() {
                Some("enabled") => CommandState::Enabled,
                Some("disabled") => CommandState::Disabled,
                _ => return Err(format!("command {name} needs enabled or disabled")),
            };
            session.set_command(*command, state);
        }
    }
    Ok(session)
}

/// The command a transport tap on the Control Center tile sends: play or pause follows the playback state.
pub fn transport_for(target: Media, playing: bool) -> Option<RemoteCommand> {
    match target {
        Media::Play if playing => Some(RemoteCommand::Pause),
        Media::Play => Some(RemoteCommand::Play),
        Media::Prev => Some(RemoteCommand::Previous),
        Media::Next => Some(RemoteCommand::Next),
        _ => None,
    }
}

/// What the shell sends back to the owner of the session.
#[derive(Clone, Debug, PartialEq)]
enum Sent {
    Command(RemoteCommand),
    Seek(f64),
}

fn body_of(sent: &Sent) -> Json {
    match sent {
        Sent::Command(c) => Json::Obj(vec![("command".to_string(), Json::Str(command_name(*c).into()))]),
        Sent::Seek(ms) => Json::Obj(vec![
            ("command".to_string(), Json::Str(command_name(RemoteCommand::SeekTo).into())),
            ("position_ms".to_string(), Json::Num(*ms)),
        ]),
    }
}

/// The shell's Now Playing state: the shown session, its owner, and what is waiting for the owner.
#[derive(Default)]
pub(crate) struct Store {
    owner: Option<String>,
    session: Option<MediaSession>,
    pending: Vec<(String, Sent)>,
}

impl Store {
    fn publish(&mut self, app_id: &str, session: MediaSession) {
        self.owner = Some(app_id.to_string());
        self.session = Some(session);
    }

    fn clear(&mut self, app_id: &str) {
        if self.owner.as_deref() == Some(app_id) {
            self.owner = None;
            self.session = None;
        }
    }

    fn current(&self) -> Option<MediaSession> {
        self.session.clone()
    }

    /// Queues `sent` for the owner when the session declares it enabled. False when nothing was queued.
    fn send(&mut self, sent: Sent) -> bool {
        let (Some(owner), Some(session)) = (self.owner.clone(), self.session.as_ref()) else {
            return false;
        };
        let command = match &sent {
            Sent::Command(c) => *c,
            Sent::Seek(_) => RemoteCommand::SeekTo,
        };
        if session.dispatch(command).is_err() {
            return false;
        }
        self.pending.push((owner, sent));
        true
    }

    /// Drops the session when its owner is no longer running.
    fn prune(&mut self, running: impl Fn(&str) -> bool) {
        if let Some(owner) = self.owner.clone() {
            if !running(&owner) {
                self.owner = None;
                self.session = None;
            }
        }
    }

    fn drain(&mut self) -> Vec<Event> {
        self.pending
            .drain(..)
            .map(|(app, sent)| Event { app: Some(app), event: "command".to_string(), body: body_of(&sent) })
            .collect()
    }
}

static STORE: Mutex<Store> = Mutex::new(Store { owner: None, session: None, pending: Vec::new() });

fn with<T>(f: impl FnOnce(&mut Store) -> T) -> Option<T> {
    let mut g = STORE.lock().ok()?;
    Some(f(&mut g))
}

/// The session Control Center shows, if an app has published one.
pub(crate) fn current() -> Option<MediaSession> {
    with(|s| s.current()).flatten()
}

/// Sends a transport command to the shown session's owner. False when there is no session or it did not enable
/// the command.
pub(crate) fn send(command: RemoteCommand) -> bool {
    with(|s| s.send(Sent::Command(command))).unwrap_or(false)
}

/// Asks the owner to seek to `position_ms`. False when there is no session or it did not enable seeking.
#[allow(dead_code)] // for the lock-screen scrubber, which is not yet routed to the service
pub(crate) fn seek(position_ms: f64) -> bool {
    with(|s| s.send(Sent::Seek(position_ms))).unwrap_or(false)
}

pub struct NowPlaying;

impl SystemService for NowPlaying {
    fn name(&self) -> &'static str {
        "now_playing"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "publish" => {
                let session = parse_session(app_id, params)?;
                with(|s| s.publish(app_id, session)).ok_or_else(|| "now playing store unavailable".to_string())?;
                Ok(Json::Null)
            }
            "clear" => {
                with(|s| s.clear(app_id)).ok_or_else(|| "now playing store unavailable".to_string())?;
                Ok(Json::Null)
            }
            _ => Err(format!("now_playing has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let apps = crate::shell::apphost::installed();
        let running = |owner: &str| {
            shell.ext.iter().enumerate().any(|(slot, host)| host.is_some() && apps.get(slot).is_some_and(|a| a.manifest.id == owner))
        };
        with(|s| {
            s.prune(running);
            s.drain()
        })
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Json {
        bifrost_core::json::parse(text).expect("valid json")
    }

    fn song() -> Json {
        body(r#"{"title":" Song ","artist":"Band","duration_ms":200000,"state":"playing","position_ms":5000,
                 "commands":{"play":"enabled","pause":"enabled","next":"disabled","seek":"enabled"}}"#)
    }

    #[test]
    fn a_publish_body_becomes_a_session_with_its_content_playback_and_commands() {
        let s = parse_session("com.example.music", &song()).expect("a session");
        assert_eq!(s.id(), "com.example.music", "the id defaults to the caller");
        let c = s.content().expect("content");
        assert_eq!(c.title, "Song");
        assert_eq!(c.subtitle(), "Band");
        assert_eq!(c.duration, MediaDuration::Finite(200.0));
        assert_eq!(s.snapshot().state, PlaybackState::Playing);
        assert_eq!(s.snapshot().position_ms, 5000);
        assert_eq!(s.command_state(RemoteCommand::Play), CommandState::Enabled);
        assert_eq!(s.command_state(RemoteCommand::Next), CommandState::Disabled);
        assert_eq!(s.command_state(RemoteCommand::Previous), CommandState::Unsupported);
    }

    #[test]
    fn a_bad_publish_is_refused_with_the_field_named() {
        assert!(parse_session("a", &body(r#"{"state":"playing"}"#)).unwrap_err().contains("title"));
        assert!(parse_session("a", &body(r#"{"title":"x","state":"rewinding"}"#)).unwrap_err().contains("rewinding"));
        assert!(parse_session("a", &body(r#"{"title":"x","commands":{"fly":"enabled"}}"#)).unwrap_err().contains("fly"));
        assert!(parse_session("a", &body(r#"{"title":"x","commands":{"play":"maybe"}}"#)).is_err());
    }

    #[test]
    fn a_live_or_unsized_session_has_no_end_and_a_negative_position_is_zero() {
        let live = parse_session("a", &body(r#"{"title":"Radio","live":true,"position_ms":-4}"#)).unwrap();
        assert_eq!(live.content().unwrap().duration, MediaDuration::Live);
        assert_eq!(live.snapshot().position_ms, 0);
        let open = parse_session("a", &body(r#"{"title":"Talk","duration_ms":0}"#)).unwrap();
        assert_eq!(open.content().unwrap().duration, MediaDuration::Continuous);
    }

    #[test]
    fn transport_taps_map_to_commands_and_play_pause_follows_the_state() {
        assert_eq!(transport_for(Media::Play, true), Some(RemoteCommand::Pause));
        assert_eq!(transport_for(Media::Play, false), Some(RemoteCommand::Play));
        assert_eq!(transport_for(Media::Prev, false), Some(RemoteCommand::Previous));
        assert_eq!(transport_for(Media::Next, true), Some(RemoteCommand::Next));
        assert_eq!(transport_for(Media::Volume, true), None);
    }

    #[test]
    fn an_enabled_command_is_queued_for_the_owner_and_a_disabled_one_is_not() {
        let mut store = Store::default();
        assert!(!store.send(Sent::Command(RemoteCommand::Play)), "no session yet");
        store.publish("app.one", parse_session("app.one", &song()).unwrap());
        assert!(store.send(Sent::Command(RemoteCommand::Pause)));
        assert!(!store.send(Sent::Command(RemoteCommand::Next)), "disabled by the owner");
        assert!(store.send(Sent::Seek(90_000.0)));
        let events = store.drain();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].app.as_deref(), Some("app.one"));
        assert_eq!(events[0].event, "command");
        assert_eq!(events[0].body.str_field("command").as_deref(), Some("pause"));
        assert_eq!(events[1].body.str_field("command").as_deref(), Some("seek"));
        assert_eq!(events[1].body.get("position_ms").and_then(Json::number), Some(90_000.0));
        assert!(store.drain().is_empty());
    }

    #[test]
    fn only_the_owner_can_clear_and_a_closed_owner_is_pruned() {
        let mut store = Store::default();
        store.publish("app.one", parse_session("app.one", &song()).unwrap());
        store.clear("app.two");
        assert!(store.current().is_some(), "another app cannot clear it");
        store.prune(|id| id == "app.two");
        assert!(store.current().is_none(), "the owner is gone, so the session goes");
        store.publish("app.one", parse_session("app.one", &song()).unwrap());
        store.clear("app.one");
        assert!(store.current().is_none());
    }

    #[test]
    fn a_new_publisher_replaces_the_shown_session() {
        let mut store = Store::default();
        store.publish("app.one", parse_session("app.one", &song()).unwrap());
        store.publish("app.two", parse_session("app.two", &body(r#"{"title":"Other","commands":{"play":"enabled"}}"#)).unwrap());
        assert_eq!(store.current().unwrap().content().unwrap().title, "Other");
        assert!(store.send(Sent::Command(RemoteCommand::Play)));
        assert_eq!(store.drain()[0].app.as_deref(), Some("app.two"));
    }
}
