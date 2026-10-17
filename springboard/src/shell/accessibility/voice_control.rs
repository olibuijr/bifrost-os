//! Voice Control: spoken command words run against the element map.
//!
//! Pure parser. The whisper recognizer (`bifrost_ui::whisper`) reports a transcript; only a final one is a command, and a
//! hypothesis the element may still revise is ignored. A command is a verb and a name: "press Save", "click Save" or
//! "tap Save" presses the element called Save, the same way `ax press` does. The shell glue is `Shell::voice_press`,
//! which resolves the name in the element map. The recognizer service (`services::recognizer`) feeds this parser while
//! the `voice_control` preference is `1`: it holds the session, and the effects a press returns go out with the next
//! shell tick. Settings has no switch for the preference yet.

use crate::shell::{Effect, Shell};
use bifrost_ui::whisper::Transcript;

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
}
