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
use std::time::Instant;

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

/// What the Now Playing tile draws: the playback state, the position now (advanced while playing) and the length
/// when the item is finite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
}

/// The position now: the published position moved on by the time since it was published while playing, and never
/// past the length. A paused or stopped session holds its position.
pub fn extrapolate(base_ms: u64, playing: bool, elapsed_ms: u64, duration_ms: Option<u64>) -> u64 {
    let now = if playing { base_ms.saturating_add(elapsed_ms) } else { base_ms };
    match duration_ms {
        Some(total) => now.min(total),
        None => now,
    }
}

/// The position a scrub to `fraction` of the bar lands on, in milliseconds (the fraction is clamped to 0 to 1).
pub fn scrub_target_ms(duration_ms: u64, fraction: f32) -> f64 {
    duration_ms as f64 * f64::from(fraction.clamp(0.0, 1.0))
}

/// The shell's Now Playing state: the shown session, its owner, when its position was last set, and what is waiting
/// for the owner.
#[derive(Default)]
pub(crate) struct Store {
    owner: Option<String>,
    session: Option<MediaSession>,
    published: Option<Instant>,
    pending: Vec<(String, Sent)>,
}

impl Store {
    fn publish(&mut self, app_id: &str, session: MediaSession) {
        self.owner = Some(app_id.to_string());
        self.session = Some(session);
        self.published = Some(Instant::now());
    }

    fn clear(&mut self, app_id: &str) {
        if self.owner.as_deref() == Some(app_id) {
            self.owner = None;
            self.session = None;
            self.published = None;
        }
    }

    fn current(&self) -> Option<MediaSession> {
        self.session.clone()
    }

    /// The shown position now, for the tile's progress bar. None without a session.
    fn progress(&self, now: Instant) -> Option<Progress> {
        let session = self.session.as_ref()?;
        let snap = session.snapshot();
        let playing = snap.state == PlaybackState::Playing;
        let elapsed = self.published.map_or(0, |t| now.saturating_duration_since(t).as_millis() as u64);
        let duration_ms = match session.content().map(|c| c.duration) {
            Some(MediaDuration::Finite(seconds)) => Some((seconds * 1000.0) as u64),
            _ => None,
        };
        Some(Progress { playing, position_ms: extrapolate(snap.position_ms, playing, elapsed, duration_ms), duration_ms })
    }

    /// Queues `sent` for the owner when the session declares it enabled. False when nothing was queued. A seek also
    /// moves the shown position at once, so the bar does not wait for the owner to publish again.
    fn send(&mut self, sent: Sent) -> bool {
        let Some(owner) = self.owner.clone() else {
            return false;
        };
        let command = match &sent {
            Sent::Command(c) => *c,
            Sent::Seek(_) => RemoteCommand::SeekTo,
        };
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.dispatch(command).is_err() {
            return false;
        }
        if let Sent::Seek(ms) = sent {
            let state = session.snapshot().state;
            session.set_playback(state, ms.max(0.0) as u64);
            self.published = Some(Instant::now());
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
                self.published = None;
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

static STORE: Mutex<Store> = Mutex::new(Store { owner: None, session: None, published: None, pending: Vec::new() });

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

/// The transport command a hardware media key asks for, by its evdev code. None for keys that are not transport keys.
/// Play/pause is a toggle; [`toggle_command`] picks the command that matches the playback state.
pub(crate) fn media_key_command(code: u16) -> Option<RemoteCommand> {
    Some(match code {
        164 => RemoteCommand::TogglePlayPause, // KEY_PLAYPAUSE
        200 | 207 => RemoteCommand::Play,      // KEY_PLAYCD, KEY_PLAY
        201 => RemoteCommand::Pause,           // KEY_PAUSECD
        166 => RemoteCommand::Stop,            // KEY_STOPCD
        163 => RemoteCommand::Next,            // KEY_NEXTSONG
        165 => RemoteCommand::Previous,        // KEY_PREVIOUSSONG
        208 => RemoteCommand::SkipForward,     // KEY_FASTFORWARD
        168 => RemoteCommand::SkipBackward,    // KEY_REWIND
        _ => return None,
    })
}

/// The command a play/pause toggle sends: pause while playing, play otherwise.
pub(crate) fn toggle_command(playing: bool) -> RemoteCommand {
    if playing {
        RemoteCommand::Pause
    } else {
        RemoteCommand::Play
    }
}

/// Sends the transport command a media key asks for to the owner of the shown session. False when the key is not a
/// transport key, there is no session, or the owner did not enable the command (the key then falls through).
pub(crate) fn media_key(code: u16) -> bool {
    let Some(command) = media_key_command(code) else {
        return false;
    };
    with(|s| {
        let command = if command == RemoteCommand::TogglePlayPause {
            let playing = s.session.as_ref().is_some_and(|x| x.snapshot().state == PlaybackState::Playing);
            toggle_command(playing)
        } else {
            command
        };
        s.send(Sent::Command(command))
    })
    .unwrap_or(false)
}

/// Asks the owner to seek to `position_ms`. False when there is no session or it did not enable seeking.
pub(crate) fn seek(position_ms: f64) -> bool {
    with(|s| s.send(Sent::Seek(position_ms))).unwrap_or(false)
}

/// The position and length the tile draws, or None without a session.
pub(crate) fn progress() -> Option<Progress> {
    with(|s| s.progress(Instant::now())).flatten()
}

/// The length in milliseconds when the shown session is finite and its owner enabled seeking; None otherwise.
pub(crate) fn seek_length_ms() -> Option<u64> {
    with(|s| {
        s.session.as_ref().filter(|x| x.dispatch(RemoteCommand::SeekTo).is_ok()).and_then(|x| match x.content().map(|c| c.duration) {
            Some(MediaDuration::Finite(seconds)) => Some((seconds * 1000.0) as u64),
            _ => None,
        })
    })
    .flatten()
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
    fn the_position_runs_on_while_playing_holds_when_paused_and_stops_at_the_end() {
        assert_eq!(extrapolate(5_000, true, 2_000, Some(200_000)), 7_000);
        assert_eq!(extrapolate(5_000, false, 2_000, Some(200_000)), 5_000, "a paused session holds its position");
        assert_eq!(extrapolate(199_000, true, 9_000, Some(200_000)), 200_000, "capped at the length");
        assert_eq!(extrapolate(1_000, true, 4_000, None), 5_000, "a live or open item has no cap");
    }

    #[test]
    fn a_seek_moves_the_shown_position_and_is_sent_to_the_owner() {
        let mut store = Store::default();
        store.publish("app.one", parse_session("app.one", &song()).unwrap());
        assert!(store.send(Sent::Seek(90_000.0)));
        assert_eq!(store.current().unwrap().snapshot().position_ms, 90_000);
        let p = store.progress(Instant::now()).expect("a session has progress");
        assert_eq!(p.duration_ms, Some(200_000));
        assert!(p.playing);
        assert!((90_000..=200_000).contains(&p.position_ms), "at least the seek target, never past the length");
        assert_eq!(store.drain()[0].body.get("position_ms").and_then(Json::number), Some(90_000.0));
    }

    #[test]
    fn media_keys_map_to_transport_commands_and_other_keys_do_not() {
        assert_eq!(media_key_command(164), Some(RemoteCommand::TogglePlayPause));
        assert_eq!(media_key_command(163), Some(RemoteCommand::Next));
        assert_eq!(media_key_command(165), Some(RemoteCommand::Previous));
        assert_eq!(media_key_command(166), Some(RemoteCommand::Stop));
        assert_eq!(media_key_command(208), Some(RemoteCommand::SkipForward));
        assert_eq!(media_key_command(168), Some(RemoteCommand::SkipBackward));
        assert_eq!(media_key_command(115), None, "volume up is the shell's");
        assert_eq!(media_key_command(28), None, "enter is not a transport key");
    }

    #[test]
    fn a_play_pause_key_pauses_while_playing_and_plays_otherwise() {
        assert_eq!(toggle_command(true), RemoteCommand::Pause);
        assert_eq!(toggle_command(false), RemoteCommand::Play);
    }

    #[test]
    fn a_scrub_lands_on_its_share_of_the_length() {
        assert_eq!(scrub_target_ms(200_000, 0.25), 50_000.0);
        assert_eq!(scrub_target_ms(200_000, 1.5), 200_000.0, "clamped to the end");
        assert_eq!(scrub_target_ms(200_000, -0.5), 0.0, "clamped to the start");
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
