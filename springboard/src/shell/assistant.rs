//! The Assistant system service: the shell decides what the assistant may see and do, in one place.
//!
//! - Settings (`assistant.*` prefs): on or off, the shortcut, the backend (off by default), history (off by default),
//!   and per-app access to App Intents.
//! - Catalogue: the intents of the installed apps the person allowed, read from their manifests (`[[intent]]`
//!   tables, see `intents.rs`). Only allowed apps are listed, and only their entries are ever sent to a backend.
//! - Protocol: the request body for the chosen backend (the Anthropic Messages format that `assistant-desktop`
//!   speaks) and the parsed reply, which is either a plain answer or one intent call checked against the catalogue.
//! - Gate: an intent call is only proposed as a confirmation card. [`execute`] runs one only after [`Gate::confirm`].
//!
//! Nothing leaves the machine until a backend is chosen. [`ask`] sends one intent request through the shared
//! `assistant-client` crate on a worker thread; no UI calls it yet (see `docs/bifrost-ui.md`, "Assistant system
//! service").

use super::intents::{self, ManifestIntent};
use super::prefs::Prefs;
use std::collections::BTreeSet;
use std::io;
use std::time::Duration;

// The intent protocol and the HTTPS client are shared with the Assistant app (`springboard/assistant-client`).
pub use assistant_client::intents::{card_text, intent_request_body, parse_param, parse_reply, reply_text, safe_id, Call, Catalogue, Entry, Param, Reply};
pub use assistant_client::{Connection, Error};

pub const KEY_ENABLED: &str = "assistant.enabled";
pub const KEY_CHORD: &str = "assistant.shortcut";
pub const KEY_BACKEND: &str = "assistant.backend";
pub const KEY_HISTORY: &str = "assistant.history";
/// Answers are read aloud through the speech service (`services::speech`). Off by default.
pub const KEY_SPEAK: &str = "assistant.speak";
/// Per-app access: `assistant.app.<app id>` is `1` when the assistant may use that app's intents.
pub const KEY_APP_PREFIX: &str = "assistant.app.";

/// Evdev code of the A key.
const KEY_A: u16 = 30;
/// Longest intent description sent to a backend, in characters.
const DESCRIPTION_MAX: usize = 300;

/// Where requests go. `Off` sends nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Off,
    Anthropic,
}

impl Backend {
    /// Every backend, in the order the Settings pop-up lists them.
    pub const ALL: [Backend; 2] = [Backend::Off, Backend::Anthropic];

    pub fn key(self) -> &'static str {
        match self {
            Backend::Off => "off",
            Backend::Anthropic => "anthropic",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Backend::Off),
            "anthropic" => Some(Backend::Anthropic),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Off => "Off",
            Backend::Anthropic => "Anthropic API",
        }
    }

    /// The host a request is sent to, or `None` when nothing is sent.
    pub fn host(self) -> Option<&'static str> {
        match self {
            Backend::Off => None,
            Backend::Anthropic => Some("api.anthropic.com"),
        }
    }
}

/// The system shortcut that opens the assistant panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    Off,
    /// Cmd+Shift+A: free in the shell's chord table (Cmd+Space is Search, Ctrl+Space the input source, and
    /// Ctrl+Option+Space reader activation). Fn/Globe is not reported on evdev keyboards, so it is not offered.
    MetaShiftA,
}

impl Default for Chord {
    fn default() -> Self {
        Chord::MetaShiftA
    }
}

impl Chord {
    pub fn key(self) -> &'static str {
        match self {
            Chord::Off => "off",
            Chord::MetaShiftA => "meta-shift-a",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Chord::Off),
            "meta-shift-a" => Some(Chord::MetaShiftA),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Chord::Off => "Off",
            Chord::MetaShiftA => "Cmd+Shift+A",
        }
    }

    /// True when the key event `code` with these modifiers is this chord.
    pub fn matches(self, code: u16, meta: bool, ctrl: bool, alt: bool, shift: bool) -> bool {
        match self {
            Chord::Off => false,
            Chord::MetaShiftA => code == KEY_A && meta && shift && !ctrl && !alt,
        }
    }
}

/// The assistant's settings. The defaults send nothing and store nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub chord: Chord,
    pub backend: Backend,
    /// Keep conversations (and send earlier turns to the backend). Off by default.
    pub history: bool,
    /// Read each answer aloud through the speech service. Off by default.
    pub speak: bool,
    /// App ids whose intents the assistant may use.
    pub allowed: BTreeSet<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: false, chord: Chord::default(), backend: Backend::Off, history: false, speak: false, allowed: BTreeSet::new() }
    }
}

impl Settings {
    pub fn load(prefs: &Prefs) -> Self {
        let d = Settings::default();
        Settings {
            enabled: prefs.get_bool(KEY_ENABLED, d.enabled),
            chord: prefs.get(KEY_CHORD).and_then(Chord::parse).unwrap_or(d.chord),
            backend: prefs.get(KEY_BACKEND).and_then(Backend::parse).unwrap_or(d.backend),
            history: prefs.get_bool(KEY_HISTORY, d.history),
            speak: prefs.get_bool(KEY_SPEAK, d.speak),
            allowed: prefs
                .entries()
                .filter_map(|(k, v)| match k.strip_prefix(KEY_APP_PREFIX) {
                    Some(app) if v == "1" => Some(app.to_string()),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Writes every setting with one save. Apps that lost access are written as `0`.
    pub fn save(&self, prefs: &mut Prefs) -> io::Result<()> {
        let flag = |on: bool| if on { "1" } else { "0" };
        let mut pairs: Vec<(String, String)> = vec![
            (KEY_ENABLED.into(), flag(self.enabled).into()),
            (KEY_CHORD.into(), self.chord.key().into()),
            (KEY_BACKEND.into(), self.backend.key().into()),
            (KEY_HISTORY.into(), flag(self.history).into()),
            (KEY_SPEAK.into(), flag(self.speak).into()),
        ];
        for app in &self.allowed {
            pairs.push((format!("{KEY_APP_PREFIX}{app}"), "1".into()));
        }
        for (key, value) in prefs.entries() {
            if let Some(app) = key.strip_prefix(KEY_APP_PREFIX) {
                if value == "1" && !self.allowed.contains(app) {
                    pairs.push((key.to_string(), "0".into()));
                }
            }
        }
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        prefs.set_many(&refs)
    }
}

/// Builds the catalogue from `(app id, app name, intents)` of the installed apps. Only apps the person allowed count.
/// An intent with an unsafe id, no title, or a malformed parameter spec is left out whole.
pub fn build<'a>(apps: impl IntoIterator<Item = (&'a str, &'a str, &'a [ManifestIntent])>, settings: &Settings) -> Catalogue {
    let mut entries = Vec::new();
    for (app_id, app_name, intents) in apps {
        if !settings.allowed.contains(app_id) {
            continue;
        }
        for intent in intents {
            if !safe_id(&intent.id) || intent.title.trim().is_empty() {
                continue;
            }
            let Some(params) = intent.params.iter().map(|p| parse_param(p)).collect::<Option<Vec<Param>>>() else { continue };
            entries.push(Entry {
                app_id: app_id.to_string(),
                app_name: app_name.to_string(),
                intent_id: intent.id.clone(),
                title: intent.title.clone(),
                description: intent.description.chars().take(DESCRIPTION_MAX).collect(),
                params,
            });
        }
    }
    Catalogue { entries }
}

/// The messages one turn sends. Earlier turns go only when history is on; otherwise the request stands alone.
pub fn turn_messages(history: bool, earlier: &[(bool, String)], text: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = if history { earlier.to_vec() } else { Vec::new() };
    out.push((true, text.to_string()));
    out
}

/// Holds the one intent call waiting for the person. Proposing never runs anything; only [`Gate::confirm`] hands a
/// call out, once.
#[derive(Debug, Default)]
pub struct Gate {
    pending: Option<Call>,
}

impl Gate {
    /// Proposes `call` in place of any card still waiting. Returns the card text.
    pub fn propose(&mut self, call: Call) -> String {
        let text = card_text(&call);
        self.pending = Some(call);
        text
    }

    pub fn pending(&self) -> Option<&Call> {
        self.pending.as_ref()
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// The person pressed Confirm: the waiting call, once. Run it with [`execute`].
    pub fn confirm(&mut self) -> Option<Call> {
        self.pending.take()
    }
}

/// Runs a confirmed call through the shell's intent path (launches the app if needed). Blocking: run it off the UI thread.
pub fn execute(call: &Call, wait: Duration) -> Result<Option<String>, String> {
    let params: Vec<(&str, &str)> = call.params.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    intents::run_with(&call.app_id, &call.intent_id, &params, wait)
}

/// Speech recognition engines the shell can drive, in order of preference.
pub const SPEECH_ENGINES: [&str; 2] = ["whisper-cli", "vosk-transcriber"];

/// The first local speech engine found in `path_var` (a `PATH`-style list), or `None`. The mic button is shown only
/// when this finds one.
pub fn speech_engine(path_var: &str) -> Option<&'static str> {
    SPEECH_ENGINES
        .into_iter()
        .find(|engine| path_var.split(':').any(|dir| std::path::Path::new(dir).join(engine).is_file()))
}

/// Asks the chosen backend for one reply to `messages` (the turn from [`turn_messages`]) with the `intents` catalogue.
/// Blocking: call it from a worker thread, never the UI thread. Nothing is sent while the backend is off; the request is
/// then understood on the device ([`assistant_client::local::match_text`], keyword matching against the intents the person
/// allowed), and text that matches no intent is [`Error::NoBackend`].
pub fn ask(settings: &Settings, connection: &Connection, messages: &[(bool, String)], intents: &Catalogue) -> Result<Reply, Error> {
    if settings.backend == Backend::Off {
        let text = messages.iter().rev().find(|(from_person, _)| *from_person).map(|(_, text)| text.as_str()).unwrap_or("");
        return assistant_client::local::match_text(text, intents).ok_or(Error::NoBackend);
    }
    assistant_client::ask(connection, messages, intents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json;

    #[test]
    fn with_the_backend_off_a_request_is_matched_on_the_device() {
        let connection = Connection { api_key: "unused".into(), model: "claude-haiku-5-5".into() };
        let off = Settings { backend: Backend::Off, ..allowed(&["n.app"]) };
        let catalogue = Catalogue {
            entries: vec![Entry {
                app_id: "n.app".into(),
                app_name: "Notes".into(),
                intent_id: "create-note".into(),
                title: "Create note".into(),
                description: String::new(),
                params: vec![parse_param("text:string").unwrap()],
            }],
        };
        let messages = turn_messages(false, &[], "create note Groceries");
        match ask(&off, &connection, &messages, &catalogue) {
            Ok(Reply::Intent(call)) => assert_eq!((call.app_id.as_str(), call.params[0].1.as_str()), ("n.app", "Groceries")),
            other => panic!("expected an intent, got {other:?}"),
        }
        let unrelated = turn_messages(false, &[], "what is the weather");
        assert_eq!(ask(&off, &connection, &unrelated, &catalogue), Err(Error::NoBackend));
    }

    #[test]
    fn ask_sends_nothing_while_the_backend_is_off() {
        let connection = Connection { api_key: "unused".into(), model: "claude-haiku-5-5".into() };
        let messages = turn_messages(false, &[], "hello");
        let off = Settings { backend: Backend::Off, ..allowed(&["n.app"]) };
        assert_eq!(ask(&off, &connection, &messages, &Catalogue::default()), Err(Error::NoBackend));
    }

    fn intent(id: &str, title: &str, params: &[&str]) -> ManifestIntent {
        ManifestIntent {
            id: id.into(),
            title: title.into(),
            description: "Makes a note".into(),
            shortcut: false,
            params: params.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn allowed(apps: &[&str]) -> Settings {
        Settings { allowed: apps.iter().map(|a| a.to_string()).collect(), ..Settings::default() }
    }

    fn sample() -> Catalogue {
        let notes = vec![intent("create-note", "Create note", &["text:string", "folder:string?"])];
        let calendar = vec![intent("add-event", "Add event", &["title:string"])];
        build([("is.olibuijr.notes", "Notes", notes.as_slice()), ("is.olibuijr.calendar", "Calendar", calendar.as_slice())], &allowed(&["is.olibuijr.notes", "is.olibuijr.calendar"]))
    }

    #[test]
    fn the_defaults_send_nothing_and_store_nothing() {
        let s = Settings::default();
        assert!(!s.enabled);
        assert_eq!(s.backend, Backend::Off);
        assert_eq!(s.backend.host(), None);
        assert!(!s.history);
        assert!(s.allowed.is_empty());
        assert_eq!(s.chord, Chord::MetaShiftA);
    }

    #[test]
    fn settings_round_trip_through_prefs() {
        let mut prefs = Prefs::memory();
        let s = Settings { enabled: true, chord: Chord::Off, backend: Backend::Anthropic, history: true, speak: true, allowed: allowed(&["a.b", "c.d"]).allowed };
        s.save(&mut prefs).unwrap();
        assert_eq!(Settings::load(&prefs), s);
        assert_eq!(prefs.get("assistant.app.a.b"), Some("1"));
    }

    #[test]
    fn revoking_an_app_clears_its_access_on_save() {
        let mut prefs = Prefs::memory();
        let mut s = allowed(&["a.b", "c.d"]);
        s.save(&mut prefs).unwrap();
        s.allowed.remove("a.b");
        s.save(&mut prefs).unwrap();
        assert_eq!(prefs.get("assistant.app.a.b"), Some("0"));
        let loaded = Settings::load(&prefs);
        assert_eq!(loaded.allowed.iter().collect::<Vec<_>>(), vec!["c.d"]);
    }

    #[test]
    fn unknown_stored_values_fall_back_to_the_defaults() {
        let mut prefs = Prefs::memory();
        prefs.set(KEY_BACKEND, "some-cloud").unwrap();
        prefs.set(KEY_CHORD, "fn").unwrap();
        let s = Settings::load(&prefs);
        assert_eq!(s.backend, Backend::Off);
        assert_eq!(s.chord, Chord::MetaShiftA);
    }

    #[test]
    fn the_shortcut_is_cmd_shift_a_only() {
        let c = Chord::MetaShiftA;
        assert!(c.matches(KEY_A, true, false, false, true));
        assert!(!c.matches(KEY_A, true, false, false, false), "Cmd+A is Select All");
        assert!(!c.matches(KEY_A, false, true, false, true), "Ctrl+Shift+A is not ours");
        assert!(!c.matches(KEY_A, true, false, true, true), "Alt is not part of it");
        assert!(!Chord::Off.matches(KEY_A, true, false, false, true));
    }

    #[test]
    fn the_catalogue_lists_only_allowed_apps_and_safe_intents() {
        let notes = vec![intent("create-note", "Create note", &["text:string"]), intent("bad id!", "Bad", &[]), intent("no-title", "", &[])];
        let other = vec![intent("secret", "Secret", &[])];
        let catalogue = build([("n.app", "Notes", notes.as_slice()), ("o.app", "Other", other.as_slice())], &allowed(&["n.app"]));
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(catalogue.entries[0].intent_id, "create-note");
        assert_eq!(catalogue.entries[0].app_name, "Notes");
        assert!(catalogue.find("o.app", "secret").is_none());
    }

    #[test]
    fn an_intent_with_a_malformed_parameter_is_left_out_whole() {
        let bad = vec![intent("ok", "Ok", &["text:string"]), intent("broken", "Broken", &["text"])];
        let catalogue = build([("n.app", "Notes", bad.as_slice())], &allowed(&["n.app"]));
        assert_eq!(catalogue.entries.iter().map(|e| e.intent_id.as_str()).collect::<Vec<_>>(), vec!["ok"]);
    }

    #[test]
    fn a_long_description_is_cut_before_it_is_sent() {
        let mut long = intent("x", "X", &[]);
        long.description = "a".repeat(1000);
        let catalogue = build([("n.app", "Notes", std::slice::from_ref(&long))], &allowed(&["n.app"]));
        assert_eq!(catalogue.entries[0].description.chars().count(), DESCRIPTION_MAX);
    }

    #[test]
    fn a_plain_answer_is_text_and_an_answer_object_is_its_text() {
        let c = sample();
        assert_eq!(parse_reply("It is sunny.", &c), Ok(Reply::Answer("It is sunny.".into())));
        assert_eq!(parse_reply("{\"answer\":\"Four.\"}", &c), Ok(Reply::Answer("Four.".into())));
        assert_eq!(parse_reply("```json\n{\"answer\":\"Four.\"}\n```", &c), Ok(Reply::Answer("Four.".into())));
        assert!(parse_reply("   ", &c).is_err());
    }

    #[test]
    fn an_intent_reply_becomes_a_call_with_checked_parameters() {
        let c = sample();
        let reply = parse_reply(
            r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"Groceries"},"summary":"Make a grocery note"}"#,
            &c,
        )
        .unwrap();
        let Reply::Intent(call) = reply else { panic!("expected an intent call") };
        assert_eq!(call.app_id, "is.olibuijr.notes");
        assert_eq!(call.intent_id, "create-note");
        assert_eq!(call.params, vec![("text".to_string(), "Groceries".to_string())]);
        assert_eq!(call.summary, "Make a grocery note");
    }

    #[test]
    fn an_unknown_intent_or_parameter_is_refused() {
        let c = sample();
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"delete-all","params":{}}"#, &c).unwrap_err().contains("not allowed"));
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"a","colour":"red"}}"#, &c).unwrap_err().contains("Unknown parameter colour"));
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":3}}"#, &c).unwrap_err().contains("not text"));
    }

    #[test]
    fn a_missing_required_parameter_is_refused_and_an_optional_one_is_not_needed() {
        let c = sample();
        assert!(matches!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"folder":"Work"}}"#, &c), Ok(Reply::Ask(q)) if q.contains("text")));
        assert!(matches!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"a"}}"#, &c), Ok(Reply::Intent(_))));
    }

    #[test]
    fn an_object_that_is_neither_answer_nor_action_is_an_error() {
        assert!(parse_reply(r#"{"hello":1}"#, &sample()).is_err());
    }

    #[test]
    fn the_card_asks_before_anything_runs() {
        let call = Call {
            app_id: "is.olibuijr.notes".into(),
            app_name: "Notes".into(),
            intent_id: "create-note".into(),
            title: "Create note".into(),
            params: vec![("text".into(), "Groceries".into())],
            summary: String::new(),
        };
        assert_eq!(card_text(&call), "Create note 'Groceries' in Notes?");
        let mut gate = Gate::default();
        assert_eq!(gate.propose(call.clone()), "Create note 'Groceries' in Notes?");
        assert_eq!(gate.pending(), Some(&call), "the call waits for the person");
    }

    #[test]
    fn the_gate_hands_out_a_call_only_once_and_only_after_proposal() {
        let call = Call { app_id: "a".into(), app_name: "A".into(), intent_id: "go".into(), title: "Go".into(), params: vec![], summary: String::new() };
        let mut gate = Gate::default();
        assert_eq!(gate.confirm(), None, "nothing proposed, nothing to run");
        gate.propose(call.clone());
        gate.cancel();
        assert_eq!(gate.confirm(), None, "a cancelled card never runs");
        gate.propose(call.clone());
        assert_eq!(gate.confirm(), Some(call));
        assert_eq!(gate.confirm(), None, "one confirm, one run");
    }

    #[test]
    fn history_off_sends_only_the_new_message() {
        let earlier = vec![(true, "first".to_string()), (false, "reply".to_string())];
        assert_eq!(turn_messages(false, &earlier, "second"), vec![(true, "second".to_string())]);
        assert_eq!(turn_messages(true, &earlier, "second").len(), 3);
    }

    #[test]
    fn the_request_carries_the_catalogue_and_no_tools() {
        let body = intent_request_body("claude-haiku-5-5", &sample(), &[(true, "Note groceries".to_string())]);
        let v = json::parse(&body).unwrap();
        assert_eq!(v.str_field("model").as_deref(), Some("claude-haiku-5-5"));
        assert!(v.get("tools").is_none());
        let system = v.str_field("system").unwrap();
        assert!(system.contains("\"intent\":\"create-note\""));
        assert!(system.contains("\"app\":\"is.olibuijr.calendar\""));
        assert!(system.contains("\"name\":\"folder\",\"type\":\"string\",\"optional\":true"));
    }

    #[test]
    fn reply_text_joins_the_text_blocks_and_reports_errors() {
        let ok = r#"{"content":[{"type":"text","text":"{\"answer\":"},{"type":"tool_use","id":"x"},{"type":"text","text":"\"Hi\"}"}]}"#;
        assert_eq!(reply_text(ok).unwrap(), "{\"answer\":\"Hi\"}");
        assert_eq!(reply_text(r#"{"type":"error","error":{"message":"bad key"}}"#), Err("bad key".into()));
        assert!(reply_text("nope").is_err());
    }

    #[test]
    fn the_speech_engine_is_found_only_when_installed() {
        let dir = std::env::temp_dir().join(format!("bifrost-assistant-speech-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_str().unwrap().to_string();
        assert_eq!(speech_engine(&path), None, "no engine, no mic button");
        std::fs::write(dir.join("vosk-transcriber"), b"").unwrap();
        assert_eq!(speech_engine(&format!("/nonexistent:{path}")), Some("vosk-transcriber"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
