//! The `now_playing` service: an app publishes its Now Playing session (metadata, playback state and the commands it
//! handles) and Control Center's media tile shows the newest one. The tile's transport taps go back to the owner as
//! `command` events, and a seek goes back as a `seek` command with the position in milliseconds.
//!
//! Calls (JSON params; the SDK builds them in `bifrost_app::now_playing`):
//! - `publish` `{id?, title, artist?, album?, type?: "audio"|"video", live?: bool, duration_ms?: number,
//!    state: "playing"|"paused"|"stopped", position_ms?: number, commands?: {name: "enabled"|"disabled"}}`
//!   replaces the shell's session with the caller's. The model is `bifrost_ui::media_session`.
//! - `clear` removes the caller's session when it is the one shown.
//! - `feedback` `{toggles: {like, dislike, bookmark: bool}, rating?: {min, max, value}}` sets the lock card's like,
//!   dislike, bookmark and rating controls for the owner's session. The user's choice goes back as a `command`
//!   event: `{"command": "like"|"dislike"|"bookmark", "active": bool}` or `{"command": "rate", "rating": number}`.
//!
//! Events to the owner: `command` with `{"command": "play"|"pause"|"stop"|"toggle"|"next"|"previous"|
//! "skip_forward"|"skip_backward"|"seek"}`, plus `"position_ms"` for `seek`. A command the owner did not enable
//! (`Unsupported` or `Disabled`) is never sent. A session whose app has closed is dropped on the next tick.
//!
//! `publish` may also name the owner's Up Next queue: `queue?: [{title, artist?, playable?: bool}]`. Media
//! Control Center's Up Next picker lists it, and a pick goes back as `{"command": "queue_select", "index": n}`
//! (the entry's place in the queue). An entry the owner marked not playable is listed and cannot be picked.
//!
//! The session lives in one process-wide store: the shell draws Control Center from it, and the service is answered
//! from the app's channel. Nothing here blocks.
use super::{Event, Shell, SystemService};
use crate::shell::music::nowplaying::Target as Media;
use bifrost_core::json::Json;
use bifrost_ui::media_content::{optional_text, MediaContent, MediaDuration, MediaType};
use bifrost_ui::media_session::{CommandState, MediaSession, PlaybackState, RemoteCommand};
#[cfg(not(test))]
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
    /// A like, dislike or bookmark toggle the user set on the lock card or Control Center: the wire name and the state.
    Toggle(&'static str, bool),
    /// A rating the user chose on the lock card.
    Rate(f64),
    /// The Up Next entry the user picked, by its place in the owner's queue.
    Queue(usize),
}

fn body_of(sent: &Sent) -> Json {
    match sent {
        Sent::Command(c) => Json::Obj(vec![("command".to_string(), Json::Str(command_name(*c).into()))]),
        Sent::Seek(ms) => Json::Obj(vec![
            ("command".to_string(), Json::Str(command_name(RemoteCommand::SeekTo).into())),
            ("position_ms".to_string(), Json::Num(*ms)),
        ]),
        Sent::Toggle(name, active) => Json::Obj(vec![
            ("command".to_string(), Json::Str((*name).into())),
            ("active".to_string(), Json::Bool(*active)),
        ]),
        Sent::Queue(index) => Json::Obj(vec![
            ("command".to_string(), Json::Str("queue_select".into())),
            ("index".to_string(), Json::Num(*index as f64)),
        ]),
        Sent::Rate(value) => Json::Obj(vec![
            ("command".to_string(), Json::Str("rate".into())),
            ("rating".to_string(), Json::Num(*value)),
        ]),
    }
}

/// The feedback an app published with its session: the three toggles (like, dislike, bookmark) and, when the item
/// has a rating command, its scale and the rating now. The app's `feedback` call sets it; a new publish clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Feedback {
    /// Like, dislike, bookmark.
    pub toggles: [bool; 3],
    pub rating: Option<Scale>,
}

/// A rating scale and the rating now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale {
    pub min: f64,
    pub max: f64,
    pub value: f64,
}

/// The toggle names on the wire, in `Feedback::toggles` order.
const TOGGLES: [&str; 3] = ["like", "dislike", "bookmark"];

/// Reads the `feedback` call's body: `{"toggles": {"like": bool, "dislike": bool, "bookmark": bool},
/// "rating"?: {"min", "max", "value"}}`. Errors name the bad field.
pub(crate) fn parse_feedback(body: &Json) -> Result<Feedback, String> {
    let Some(toggles_json) = body.get("toggles") else {
        return Err("feedback needs toggles".to_string());
    };
    let mut toggles = [false; 3];
    for (slot, name) in TOGGLES.iter().enumerate() {
        toggles[slot] = match toggles_json.get(name) {
            Some(Json::Bool(b)) => *b,
            _ => return Err(format!("feedback toggle {name} needs true or false")),
        };
    }
    let rating = match body.get("rating") {
        None => None,
        Some(r) => {
            let num = |key: &str| r.get(key).and_then(Json::number).filter(|v| v.is_finite());
            let (Some(min), Some(max), Some(value)) = (num("min"), num("max"), num("value")) else {
                return Err("rating needs min, max and value".to_string());
            };
            if max <= min || value < min || value > max {
                return Err("rating is outside its scale".to_string());
            }
            Some(Scale { min, max, value })
        }
    };
    Ok(Feedback { toggles, rating })
}

/// The wire name of toggle slot `slot`, or None past the three toggles.
pub(crate) fn toggle_name(slot: usize) -> Option<&'static str> {
    TOGGLES.get(slot).copied()
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
    feedback: Option<Feedback>,
    pending: Vec<(String, Sent)>,
    /// The owner's Up Next queue, replaced by each publish.
    queue: Vec<QueueItem>,
}

/// One entry of an app's Up Next queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub title: String,
    pub artist: String,
    /// False for an entry the owner cannot play now: listed, not pickable.
    pub playable: bool,
}

/// The most entries a queue may hold; a longer list is refused.
const QUEUE_MAX: usize = 200;

/// Reads the optional `queue` of a publish body. A missing queue is an empty one.
pub fn parse_queue(body: &Json) -> Result<Vec<QueueItem>, String> {
    let Some(entries) = body.get("queue") else {
        return Ok(Vec::new());
    };
    let entries = entries.as_array().ok_or_else(|| "queue must be an array".to_string())?;
    if entries.len() > QUEUE_MAX {
        return Err(format!("queue holds at most {QUEUE_MAX} entries"));
    }
    entries
        .iter()
        .map(|entry| {
            let title = entry.str_field("title").and_then(|t| optional_text(&t)).ok_or_else(|| "each queue entry needs a title".to_string())?;
            let artist = entry.str_field("artist").and_then(|t| optional_text(&t)).unwrap_or_default();
            let playable = !matches!(entry.get("playable"), Some(Json::Bool(false)));
            Ok(QueueItem { title, artist, playable })
        })
        .collect()
}

impl Store {
    fn publish(&mut self, app_id: &str, session: MediaSession, queue: Vec<QueueItem>) {
        self.owner = Some(app_id.to_string());
        self.session = Some(session);
        self.published = Some(Instant::now());
        self.feedback = None;
        self.queue = queue;
    }

    /// Queues the owner's pick of an Up Next entry. False when there is no owner, no such entry, or it is not playable.
    fn request_queue(&mut self, index: usize) -> bool {
        let Some(owner) = self.owner.clone() else {
            return false;
        };
        if !self.queue.get(index).is_some_and(|item| item.playable) {
            return false;
        }
        self.pending.push((owner, Sent::Queue(index)));
        true
    }

    /// Sets the feedback of the shown session. Only its owner may set it.
    fn set_feedback(&mut self, app_id: &str, feedback: Feedback) -> bool {
        if self.owner.as_deref() != Some(app_id) {
            return false;
        }
        self.feedback = Some(feedback);
        true
    }

    /// Records a toggle or a rating the user set and queues it for the owner. False when the owner published no
    /// feedback, or the rating is outside its scale (a toggle is always accepted once feedback exists).
    fn request_feedback(&mut self, sent: Sent) -> bool {
        let Some(owner) = self.owner.clone() else {
            return false;
        };
        let Some(feedback) = self.feedback.as_mut() else {
            return false;
        };
        match sent {
            Sent::Toggle(name, active) => {
                let Some(slot) = TOGGLES.iter().position(|t| *t == name) else {
                    return false;
                };
                feedback.toggles[slot] = active;
            }
            Sent::Rate(value) => {
                let Some(scale) = feedback.rating.as_mut() else {
                    return false;
                };
                if !(value >= scale.min && value <= scale.max) {
                    return false;
                }
                scale.value = value;
            }
            _ => return false,
        }
        self.pending.push((owner, sent));
        true
    }

    fn clear(&mut self, app_id: &str) {
        if self.owner.as_deref() == Some(app_id) {
            self.owner = None;
            self.session = None;
            self.published = None;
            self.feedback = None;
            self.queue.clear();
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
            Sent::Toggle(..) | Sent::Rate(_) | Sent::Queue(_) => return false,
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
                self.feedback = None;
                self.queue.clear();
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

#[cfg(not(test))]
static STORE: Mutex<Store> =
    Mutex::new(Store { owner: None, session: None, published: None, feedback: None, pending: Vec::new(), queue: Vec::new() });

#[cfg(not(test))]
fn with<T>(f: impl FnOnce(&mut Store) -> T) -> Option<T> {
    let mut g = STORE.lock().ok()?;
    Some(f(&mut g))
}

// Under test each test thread has its own store. The store is process-wide so that the shell and the app channel see one
// session, but the tests run in parallel: every Shell that ticks its services runs `NowPlaying::tick`, which prunes a
// session whose owner is not a running app, so a test that published one for a made-up owner lost it to whichever other
// test ticked first. A test drives its shell on its own thread, so a per-thread store keeps the code path and drops the race.
#[cfg(test)]
thread_local! {
    static STORE: std::cell::RefCell<Store> =
        const { std::cell::RefCell::new(Store { owner: None, session: None, published: None, feedback: None, pending: Vec::new(), queue: Vec::new() }) };
}

#[cfg(test)]
fn with<T>(f: impl FnOnce(&mut Store) -> T) -> Option<T> {
    STORE.with(|s| s.try_borrow_mut().ok().map(|mut g| f(&mut g)))
}

/// The session Control Center shows, if an app has published one.
pub(crate) fn current() -> Option<MediaSession> {
    with(|s| s.current()).flatten()
}

/// The shown owner's Up Next queue (empty without a session or a queue).
pub(crate) fn up_next() -> Vec<QueueItem> {
    with(|s| s.queue.clone()).unwrap_or_default()
}

/// Sends the user's pick of an Up Next entry to the owner. False when the entry cannot be picked.
pub(crate) fn choose_up_next(index: usize) -> bool {
    with(|s| s.request_queue(index)).unwrap_or(false)
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

/// The feedback the shown session's owner published, for the lock card's like, dislike, bookmark and rating controls.
pub(crate) fn feedback() -> Option<Feedback> {
    with(|s| s.feedback).flatten()
}

/// The user set toggle `slot` (0 like, 1 dislike, 2 bookmark) to `active`. The owner gets it as a `command` event.
/// False when the owner published no feedback or the slot is unknown.
pub(crate) fn set_toggle(slot: usize, active: bool) -> bool {
    let Some(name) = toggle_name(slot) else {
        return false;
    };
    with(|s| s.request_feedback(Sent::Toggle(name, active))).unwrap_or(false)
}

/// The user chose `value` on the rating scale. False when the owner published no rating or the value is outside it.
pub(crate) fn set_rating(value: f64) -> bool {
    with(|s| s.request_feedback(Sent::Rate(value))).unwrap_or(false)
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
                let queue = parse_queue(params)?;
                with(|s| s.publish(app_id, session, queue)).ok_or_else(|| "now playing store unavailable".to_string())?;
                Ok(Json::Null)
            }
            "clear" => {
                with(|s| s.clear(app_id)).ok_or_else(|| "now playing store unavailable".to_string())?;
                Ok(Json::Null)
            }
            "feedback" => {
                let feedback = parse_feedback(params)?;
                let owned = with(|s| s.set_feedback(app_id, feedback)).ok_or_else(|| "now playing store unavailable".to_string())?;
                if !owned {
                    return Err("only the owner of the shown session may set its feedback".to_string());
                }
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
    fn a_publish_queue_is_read_and_a_pick_is_sent_for_a_playable_entry_only() {
        let q = parse_queue(&body(r#"{"queue":[{"title":"A","artist":"X"},{"title":"B","playable":false},{"title":"C"}]}"#)).expect("a queue");
        assert_eq!(q.len(), 3);
        assert_eq!((q[0].artist.as_str(), q[0].playable), ("X", true));
        assert!(!q[1].playable && q[2].artist.is_empty());
        assert!(parse_queue(&body(r#"{"title":"x"}"#)).unwrap().is_empty(), "no queue is an empty one");
        assert!(parse_queue(&body(r#"{"queue":[{"artist":"no title"}]}"#)).unwrap_err().contains("title"));
        assert!(parse_queue(&body(r#"{"queue":"nope"}"#)).is_err());
        let mut store = Store { owner: Some("app".into()), session: None, published: None, feedback: None, pending: Vec::new(), queue: q };
        assert!(store.request_queue(0), "a playable entry is picked");
        assert!(!store.request_queue(1), "an unplayable entry is not");
        assert!(!store.request_queue(9), "an unknown entry is not");
        let events = store.drain();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].body.get("command"), Some(Json::Str(c)) if c == "queue_select"));
        assert!(matches!(events[0].body.get("index"), Some(Json::Num(n)) if *n == 0.0));
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
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
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
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
        store.clear("app.two");
        assert!(store.current().is_some(), "another app cannot clear it");
        store.prune(|id| id == "app.two");
        assert!(store.current().is_none(), "the owner is gone, so the session goes");
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
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
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
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
    fn feedback_parses_the_toggles_and_the_rating_scale() {
        let fb = parse_feedback(&body(r#"{"toggles":{"like":true,"dislike":false,"bookmark":false},"rating":{"min":0,"max":5,"value":3}}"#)).unwrap();
        assert_eq!(fb.toggles, [true, false, false]);
        assert_eq!(fb.rating, Some(Scale { min: 0.0, max: 5.0, value: 3.0 }));
        let bare = parse_feedback(&body(r#"{"toggles":{"like":false,"dislike":false,"bookmark":true}}"#)).unwrap();
        assert_eq!(bare.rating, None, "no rating command, no scale");
    }

    #[test]
    fn bad_feedback_is_refused_with_the_field_named() {
        assert!(parse_feedback(&body(r#"{"rating":{"min":0,"max":5,"value":1}}"#)).unwrap_err().contains("toggles"));
        assert!(parse_feedback(&body(r#"{"toggles":{"like":"yes","dislike":false,"bookmark":false}}"#)).unwrap_err().contains("like"));
        assert!(parse_feedback(&body(r#"{"toggles":{"like":true,"dislike":false,"bookmark":false},"rating":{"min":5,"max":5,"value":5}}"#)).is_err());
        assert!(parse_feedback(&body(r#"{"toggles":{"like":true,"dislike":false,"bookmark":false},"rating":{"min":0,"max":5,"value":9}}"#)).is_err());
    }

    #[test]
    fn the_owner_sets_feedback_and_the_user_toggles_and_rates_it() {
        let mut store = Store::default();
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
        let fb = Feedback { toggles: [false; 3], rating: Some(Scale { min: 0.0, max: 5.0, value: 0.0 }) };
        assert!(!store.request_feedback(Sent::Toggle("like", true)), "no feedback published yet");
        assert!(store.set_feedback("app.one", fb));
        assert!(!store.set_feedback("app.two", fb), "only the owner may set feedback");
        assert!(store.request_feedback(Sent::Toggle("like", true)));
        assert!(!store.request_feedback(Sent::Rate(9.0)), "a rating outside the scale is refused");
        assert!(store.request_feedback(Sent::Rate(4.0)));
        let events = store.drain();
        assert_eq!(events[0].body.str_field("command").as_deref(), Some("like"));
        assert_eq!(events[0].body.get("active"), Some(&Json::Bool(true)));
        assert_eq!(events[1].body.str_field("command").as_deref(), Some("rate"));
        assert_eq!(events[1].body.get("rating").and_then(Json::number), Some(4.0));
    }

    #[test]
    fn a_new_publish_clears_the_previous_feedback() {
        let mut store = Store::default();
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
        store.set_feedback("app.one", Feedback { toggles: [true; 3], rating: None });
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
        assert_eq!(store.feedback, None);
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
        store.publish("app.one", parse_session("app.one", &song()).unwrap(), Vec::new());
        store.publish("app.two", parse_session("app.two", &body(r#"{"title":"Other","commands":{"play":"enabled"}}"#)).unwrap(), Vec::new());
        assert_eq!(store.current().unwrap().content().unwrap().title, "Other");
        assert!(store.send(Sent::Command(RemoteCommand::Play)));
        assert_eq!(store.drain()[0].app.as_deref(), Some("app.two"));
    }
}
