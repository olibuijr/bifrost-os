//! Voice Control: spoken command words run against the element map.
//!
//! The parser is pure. The whisper recognizer (`bifrost_ui::whisper`) reports a transcript; only a final one is a command, and a
//! hypothesis the element may still revise is ignored. A command is a verb and a name: "press Save", "click Save" or
//! "tap Save" presses the element called Save, the same way `ax press` does. The shell glue is `Shell::voice_press`,
//! which resolves the name in the element map. The shell keeps one recognition running in the `speech_recognizer`
//! service while Voice Control is on (`Shell::voice_control_tick`): it starts when the setting is switched on, is polled for
//! final transcripts, and ends when the setting is switched off. The service refuses without the person's consent and the
//! installed model (Settings > Keyboard > Dictation); the refusal is announced once and Voice Control tries again later.

use crate::shell::{Effect, Shell};
use bifrost_core::json::Json;
use bifrost_proto::AnnouncePriority;
use bifrost_ui::whisper::Transcript;

/// The client name Voice Control's recognition runs under in the service. The service leaves a session of this prefix alone
/// when no installed app of that name runs (see `services::recognizer`).
pub const SESSION_APP: &str = "shell:voice-control";
/// Milliseconds between two polls for transcripts.
pub const POLL_MS: f64 = 250.0;
/// Milliseconds before a refused or ended recognition is started again.
pub const RETRY_MS: f64 = 15_000.0;

/// Voice Control's recognition as the shell keeps it.
#[derive(Debug, Default)]
pub struct Session {
    /// a recognition of this client is running in the service
    running: bool,
    /// the earliest time of the next poll or the next start
    next_ms: f64,
    /// the person was told why Voice Control cannot listen, since it was last switched on
    told: bool,
}

/// What a `poll` reply holds: the final transcripts, and whether the recognition still runs.
pub fn replies(reply: &Json) -> (Vec<Transcript>, bool) {
    let finals = reply
        .get("transcripts")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .filter(|t| matches!(t.get("final"), Some(Json::Bool(true))))
        .filter_map(|t| t.str_field("text"))
        .map(|text| Transcript { text, is_final: true })
        .collect();
    let running = matches!(reply.get("running"), Some(Json::Bool(true)));
    (finals, running)
}

/// One command a spoken transcript asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Press the element whose name is `name` (the words after the verb, trimmed and lower-cased).
    Press(String),
}

/// Verbs that press an element, in the words the person may say.
const PRESS_VERBS: [&str; 3] = ["press", "click", "tap"];

/// The command in a transcript. None for a hypothesis (not final), a blank transcript, a verb it does not know, or a verb
/// with no name after it. Trailing punctuation the recognizer adds ("Save.") is dropped.
pub fn parse(transcript: &Transcript) -> Option<Command> {
    if !transcript.is_final {
        return None;
    }
    let words = transcript.text.trim().trim_end_matches(['.', '!', '?']).trim();
    let (verb, name) = words.split_once(char::is_whitespace)?;
    if !PRESS_VERBS.contains(&verb.to_lowercase().as_str()) {
        return None;
    }
    let name = name.trim();
    (!name.is_empty()).then(|| Command::Press(name.to_lowercase()))
}

impl Shell {
    /// Whether Voice Control is on (Settings > Accessibility > Touch > Voice Control).
    pub(in crate::shell) fn voice_control_on(&self) -> bool {
        self.settings.as_ref().map_or_else(|| super::AccessibilitySettings::load(&self.prefs).voice_control, |st| st.accessibility().voice_control)
    }

    /// One shell tick of Voice Control. Switched on, it starts a recognition in the `speech_recognizer` service (a refusal is
    /// announced once and tried again after `RETRY_MS`), polls it every `POLL_MS` and runs each final transcript as a command.
    /// Switched off, it ends the recognition. The effects are those of the commands that ran.
    pub(in crate::shell) fn voice_control_tick(&mut self, t: f64) -> Vec<Effect> {
        let on = self.voice_control_on();
        if !on {
            if self.voice.running {
                let _ = self.service_call(SESSION_APP, "speech_recognizer", "stop", "", t);
                self.voice.running = false;
            }
            self.voice.told = false;
            return Vec::new();
        }
        if t < self.voice.next_ms {
            return Vec::new();
        }
        if !self.voice.running {
            self.voice.next_ms = t + RETRY_MS;
            match self.service_call(SESSION_APP, "speech_recognizer", "start", "{\"locale\":\"en_US\"}", t) {
                Ok(_) => {
                    self.voice.running = true;
                    self.voice.next_ms = t + POLL_MS;
                    self.voice.told = false;
                }
                Err(e) => {
                    if !self.voice.told {
                        self.voice.told = true;
                        self.announce.post(&format!("Voice Control cannot listen: {e}"), AnnouncePriority::High);
                        self.dirty = true;
                    }
                }
            }
            return Vec::new();
        }
        self.voice.next_ms = t + POLL_MS;
        let reply = match self.service_call(SESSION_APP, "speech_recognizer", "poll", "", t) {
            Ok(r) => r,
            Err(_) => {
                self.voice.running = false;
                return Vec::new();
            }
        };
        let (finals, running) = replies(&reply);
        if !running {
            // the pipeline ended: start again after the pause
            self.voice.running = false;
            self.voice.next_ms = t + RETRY_MS;
        }
        let mut effects = Vec::new();
        for transcript in finals {
            match self.voice_press(&transcript, t) {
                Some(Ok((_, fx))) => {
                    effects.extend(fx);
                    self.dirty = true;
                }
                Some(Err(e)) => {
                    self.announce.post(&e, AnnouncePriority::High);
                    self.dirty = true;
                }
                None => {}
            }
        }
        effects
    }

    /// Runs a final transcript as a Voice Control command. None when the transcript is not a command; otherwise the
    /// result of pressing the named element (its address and effects, or why it could not be pressed), as `ax press`
    /// answers it.
    pub(in crate::shell) fn voice_press(&mut self, transcript: &Transcript, t: f64) -> Option<Result<(String, Vec<Effect>), String>> {
        match parse(transcript)? {
            Command::Press(name) => Some(self.ax_press(&name, t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(text: &str) -> Transcript {
        Transcript { text: text.into(), is_final: true }
    }

    #[test]
    fn a_press_verb_names_the_element_to_press() {
        assert_eq!(parse(&fin("press Save")), Some(Command::Press("save".into())));
        assert_eq!(parse(&fin("Click Software Update")), Some(Command::Press("software update".into())));
        assert_eq!(parse(&fin("  tap   Close window  ")), Some(Command::Press("close window".into())));
    }

    #[test]
    fn trailing_punctuation_from_the_recognizer_is_dropped() {
        assert_eq!(parse(&fin("Press Save.")), Some(Command::Press("save".into())));
        assert_eq!(parse(&fin("tap Done!")), Some(Command::Press("done".into())));
    }

    #[test]
    fn a_hypothesis_is_never_a_command() {
        let partial = Transcript { text: "press Save".into(), is_final: false };
        assert_eq!(parse(&partial), None);
    }

    #[test]
    fn unknown_verbs_and_missing_names_are_not_commands() {
        assert_eq!(parse(&fin("open Settings")), None);
        assert_eq!(parse(&fin("press")), None, "a verb with no name");
        assert_eq!(parse(&fin("press   ")), None);
        assert_eq!(parse(&fin("")), None);
        assert_eq!(parse(&fin("pressing Save")), None, "the verb must be a whole word");
    }

    #[test]
    fn a_poll_reply_gives_the_final_transcripts_and_whether_the_recognition_runs() {
        let reply = bifrost_core::json::parse(r#"{"transcripts":[{"text":"press Save","final":true},{"text":"press Sa","final":false},{"text":"tap Open","final":true}],"running":true}"#).unwrap();
        let (finals, running) = replies(&reply);
        assert_eq!(finals.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["press Save", "tap Open"]);
        assert!(finals.iter().all(|t| t.is_final) && running);
        let ended = bifrost_core::json::parse(r#"{"transcripts":[],"running":false}"#).unwrap();
        let (none, running) = replies(&ended);
        assert!(none.is_empty() && !running);
        let (garbled, running) = replies(&Json::Null);
        assert!(garbled.is_empty() && !running, "a reply with no fields is nothing");
    }
}
