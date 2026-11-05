//! Spoken Content: Speak Selection (Option+Escape) reads the selected text of the focused field aloud through the speech
//! engine, with or without the screen reader on. The selection comes from the app's `ToShell::TextState`, the surrounding
//! text and selection of the focused field, so only text fields can be spoken. Speak Items Under the Pointer lives in
//! `hover_speech`. The pure part (`selected_text`) is tested on the host; the `Shell`
//! method is the glue.

use crate::shell::Shell;
use bifrost_proto::TextState;

/// The selected text of a field: the bytes between the anchor and the caret, both UTF-8 offsets into `text`
/// (`TextState`). Empty when nothing is selected, or when an offset is out of range or splits a character.
pub fn selected_text(t: &TextState) -> String {
    t.selected_text().to_string()
}

impl Shell {
    /// Option+Escape: speaks the selection of the focused field. Nothing is spoken when the front app has no focused field
    /// with a selection, or no speech engine is installed.
    pub(in crate::shell) fn speak_selection(&mut self) {
        let front = matches!(self.screen, crate::shell::Screen::App(crate::shell::apps::AppId::External(_)))
            .then(|| self.ext_fg.and_then(|n| self.ext.get(n as usize)).and_then(Option::as_ref))
            .flatten();
        let text = front.map(|h| selected_text(&h.text_state)).unwrap_or_default();
        if text.trim().is_empty() {
            return;
        }
        let config = self.speech_config();
        self.speech.configure(config);
        self.speech.say(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str, cursor: u32, anchor: u32) -> TextState {
        TextState { text: text.into(), cursor, anchor, marked: None }
    }

    #[test]
    fn the_selection_is_the_text_between_anchor_and_caret_in_either_order() {
        assert_eq!(selected_text(&state("hello world", 5, 0)), "hello");
        assert_eq!(selected_text(&state("hello world", 0, 5)), "hello", "a backward selection reads the same");
        assert_eq!(selected_text(&state("hello world", 11, 6)), "world");
    }

    #[test]
    fn no_selection_speaks_nothing() {
        assert_eq!(selected_text(&state("hello", 3, 3)), "");
        assert_eq!(selected_text(&TextState::default()), "");
    }

    #[test]
    fn offsets_are_utf8_bytes_and_a_split_character_speaks_nothing() {
        // "é" is two bytes: bytes 0..2 are the whole letter, 0..1 splits it
        assert_eq!(selected_text(&state("éa", 2, 0)), "é");
        assert_eq!(selected_text(&state("éa", 1, 0)), "");
        assert_eq!(selected_text(&state("ab", 9, 0)), "", "an offset past the text is refused");
    }
}
