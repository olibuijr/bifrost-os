//! Spoken replies to voice commands. After a command is resolved, the interface answers the ones that need an answer:
//! a click that found its control, a name that matched more than one control, a name that matched nothing, and a
//! phrase that is not a command. Actions that are silent on screen (scrolling, showing an overlay) get no reply. The
//! reply text is queued on the speech synthesizer with the voice the user chose.

use crate::speech::{Synthesizer, Utterance};
use crate::voice_commands::{Command, Resolution, Target};

/// The sentence the interface speaks for `command`, or `None` when the command needs no spoken answer. `resolution`
/// is the outcome of resolving a click; other commands ignore it.
pub fn spoken_reply(command: &Command, resolution: &Resolution, targets: &[Target]) -> Option<String> {
    match command {
        Command::Click(name) => match resolution {
            Resolution::Matched(id) => targets.iter().find(|t| t.id == *id).map(|t| format!("Clicked {}.", t.name)),
            Resolution::Ambiguous(_) => Some(format!("More than one item is called {name}. Say its number.")),
            Resolution::NotFound => Some(format!("I cannot find {name}.")),
        },
        Command::Unknown => Some("Sorry, I did not understand that.".to_string()),
        Command::Dictate(_) | Command::Key(_) | Command::Scroll(_) | Command::Back | Command::Show(_) | Command::Hide(_) => None,
    }
}

/// The utterance that speaks `reply` in the language `language` (a BCP 47 tag such as `en-GB`).
pub fn reply_utterance(reply: &str, language: &str) -> Utterance {
    Utterance::new(reply).with_language(language)
}

/// Queues the reply for `command` on `synthesizer`. Returns the text queued, or `None` when nothing was spoken.
pub fn speak_reply(synthesizer: &mut Synthesizer, command: &Command, resolution: &Resolution, targets: &[Target], language: &str) -> Option<String> {
    let reply = spoken_reply(command, resolution, targets)?;
    synthesizer.speak(reply_utterance(&reply, language));
    Some(reply)
}
