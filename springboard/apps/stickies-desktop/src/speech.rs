//! Edit > Speech: reads a note aloud through the system speech engine (`espeak-ng`, else `espeak`), the same
//! engine the `say` tool runs. The text goes to the engine on its standard input. Speech starts only from the menu
//! and stops with Stop Speaking, a new Start Speaking or when the app quits.

use crate::model::Note;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::cell::RefCell;
use std::io::Write;
use std::process::{Child, Command, Stdio};

/// The engines tried in order: the packaged one first, then the older name.
pub const ENGINES: &[&str] = &["espeak-ng", "espeak"];

#[derive(Debug, PartialEq, Eq)]
pub enum SpeakError {
    /// There is nothing but blank space to read.
    NoText,
    /// None of the engines is installed.
    NoEngine,
}

/// The text Start Speaking reads: the selection, or the whole note when nothing is selected. `None` when blank.
pub fn text_to_speak(note: &Note) -> Option<String> {
    let text = if note.has_selection() { note.selected_text() } else { note.text.as_str() };
    let text = clean(text);
    (!text.is_empty()).then_some(text)
}

/// Control characters other than line breaks become spaces, and the ends are trimmed.
pub fn clean(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

/// The running engine process, if any.
#[derive(Default)]
pub struct Speaker {
    child: RefCell<Option<Child>>,
}

impl Speaker {
    /// Speaks `text`, ending any speech in progress first.
    pub fn start(&self, text: &str) -> Result<(), SpeakError> {
        self.start_with(ENGINES, text)
    }

    /// Like [`Speaker::start`] with the engines to try.
    pub fn start_with(&self, engines: &[&str], text: &str) -> Result<(), SpeakError> {
        let text = clean(text);
        if text.is_empty() {
            return Err(SpeakError::NoText);
        }
        self.stop();
        for engine in engines {
            let spawned = Command::new(engine).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            let Ok(mut child) = spawned else { continue };
            if let Some(mut stdin) = child.stdin.take() {
                // Utility dispatch writes so a long note cannot block the app; killing the child closes the pipe.
                let bytes = text.as_bytes().to_vec();
                DispatchQueue::global(Qos::Utility).async_(move || {
                    let _ = stdin.write_all(&bytes);
                });
            }
            *self.child.borrow_mut() = Some(child);
            return Ok(());
        }
        Err(SpeakError::NoEngine)
    }

    /// Ends the speech. True when the engine was still speaking.
    pub fn stop(&self) -> bool {
        let taken = self.child.borrow_mut().take();
        let Some(mut child) = taken else { return false };
        let running = matches!(child.try_wait(), Ok(None));
        let _ = child.kill();
        let _ = child.wait();
        running
    }

    /// True while the engine process runs. A finished process is collected here.
    pub fn is_speaking(&self) -> bool {
        let mut slot = self.child.borrow_mut();
        let state = slot.as_mut().map(|child| child.try_wait());
        match state {
            Some(Ok(None)) => true,
            Some(_) => {
                *slot = None;
                false
            }
            None => false,
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str) -> Note {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.set_text(text);
        n
    }

    #[test]
    fn a_note_with_a_selection_reads_only_the_selection() {
        let mut n = note("one two three");
        assert_eq!(text_to_speak(&n).as_deref(), Some("one two three"));
        n.set_caret(4, false);
        n.set_caret(7, true);
        assert_eq!(text_to_speak(&n).as_deref(), Some("two"));
    }

    #[test]
    fn blank_notes_and_blank_selections_read_nothing() {
        assert_eq!(text_to_speak(&note("  \n ")), None);
        assert_eq!(text_to_speak(&note("")), None);
        let mut n = note("a   b");
        n.set_caret(1, false);
        n.set_caret(4, true);
        assert_eq!(text_to_speak(&n), None);
    }

    #[test]
    fn clean_keeps_line_breaks_and_replaces_other_control_characters() {
        assert_eq!(clean("  a\tb\u{7}c\nd \n"), "a b c\nd");
    }

    #[test]
    fn a_missing_engine_is_reported_and_nothing_runs() {
        let speaker = Speaker::default();
        assert_eq!(speaker.start_with(&["bifrost-no-such-speech-engine"], "hello"), Err(SpeakError::NoEngine));
        assert!(!speaker.is_speaking());
        assert!(!speaker.stop());
    }

    #[test]
    fn blank_text_is_refused_before_any_engine_is_tried() {
        let speaker = Speaker::default();
        assert_eq!(speaker.start_with(&["bifrost-no-such-speech-engine"], " \n"), Err(SpeakError::NoText));
    }
}
