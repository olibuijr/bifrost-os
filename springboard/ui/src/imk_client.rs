//! The text input client an input method talks to (the client protocol of the input method kit): the calls an input
//! method makes to insert text, set and clear marked text, and ask for the selection, the marked range and the
//! attributes of a character. [`apply`] turns what an [`InputController`] reports for an event into those calls, so a
//! text view only implements [`TextInputClient`]. Ranges are UTF-8 byte ranges in the client's text, as in `text.rs`.

use std::ops::Range;

use crate::input_method::{Composition, InputController, Outcome};

/// What an input method may ask about a character.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterAttributes {
    /// Line height rectangle as (x, y, width, height).
    pub line_height_rect: (f32, f32, f32, f32),
}

/// The client side of a session.
pub trait TextInputClient {
    /// Inserts final text. `replacement` is the range it replaces, or None for the current selection.
    fn insert_text(&mut self, text: &str, replacement: Option<Range<usize>>);
    /// Shows marked text with `selection` selected inside it; it replaces `replacement` or the current marked text.
    fn set_marked_text(&mut self, text: &str, selection: Range<usize>, replacement: Option<Range<usize>>);
    fn selected_range(&self) -> Range<usize>;
    fn marked_range(&self) -> Option<Range<usize>>;
    fn attributes_for_character_index(&self, index: usize) -> Option<CharacterAttributes>;
}

/// Shows the controller's composition in the client as marked text, with the caret inside it.
pub fn update_composition(controller: &InputController, client: &mut dyn TextInputClient) {
    if let Some(c) = controller.composition() {
        let composed = controller.composed_string().unwrap_or_default();
        let caret = c.caret().min(composed.len());
        let replacement = client.marked_range();
        client.set_marked_text(composed, caret..caret, replacement);
    }
}

/// Applies an outcome to the client. Returns true when the client must still handle the event itself.
pub fn apply(outcome: &Outcome, client: &mut dyn TextInputClient) -> bool {
    match outcome {
        Outcome::Consumed => false,
        Outcome::PassThrough => true,
        Outcome::Committed(text) | Outcome::Cancelled(text) => {
            let replacement = client.marked_range();
            client.insert_text(text, replacement);
            false
        }
    }
}

/// The editor is a text input client as it is: an input method's marked text and commits are the editor's own
/// edits (one undo step per composition), and the ranges are the editor's byte ranges.
impl TextInputClient for crate::text::TextEditor {
    fn insert_text(&mut self, text: &str, replacement: Option<Range<usize>>) {
        match replacement {
            // a commit over some other range than the composition replaces that range, as one edit
            Some(r) if self.buf.marked_range().as_ref() != Some(&r) => {
                self.buf.cancel_marked();
                let end = r.end.min(self.buf.len());
                let start = r.start.min(end);
                self.replace_range(start..end, text, "Typing");
            }
            // a commit over the marked range (or the selection) is the editor's own insert
            _ => {
                crate::text::TextEditor::insert_text(self, text);
            }
        }
    }

    fn set_marked_text(&mut self, text: &str, selection: Range<usize>, replacement: Option<Range<usize>>) {
        let start = selection.start.min(text.len());
        let len = selection.end.saturating_sub(selection.start).min(text.len() - start);
        crate::text::TextEditor::set_marked_text(self, text, start, len, replacement);
    }

    fn selected_range(&self) -> Range<usize> {
        self.buf.selection()
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        self.buf.marked_range()
    }

    fn attributes_for_character_index(&self, _index: usize) -> Option<CharacterAttributes> {
        // the editor holds text, not layout: the view that draws it knows the line rectangle
        None
    }
}

/// The marked range of a composition, for a client that keeps no range of its own.
pub fn marked_range_of(c: &Composition) -> Range<usize> {
    c.marked_range()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_method::{Event, Registration, Server};
    use crate::keys::{Key, KeyCombo};

    #[derive(Default)]
    struct Mock {
        text: String,
        marked: Option<Range<usize>>,
        calls: Vec<String>,
    }

    impl TextInputClient for Mock {
        fn insert_text(&mut self, text: &str, replacement: Option<Range<usize>>) {
            let r = replacement.unwrap_or(self.text.len()..self.text.len());
            self.text.replace_range(r, text);
            self.marked = None;
            self.calls.push(format!("insert {text}"));
        }
        fn set_marked_text(&mut self, text: &str, _selection: Range<usize>, replacement: Option<Range<usize>>) {
            let r = replacement.unwrap_or(self.text.len()..self.text.len());
            let start = r.start;
            self.text.replace_range(r, text);
            self.marked = Some(start..start + text.len());
            self.calls.push(format!("mark {text}"));
        }
        fn selected_range(&self) -> Range<usize> {
            self.text.len()..self.text.len()
        }
        fn marked_range(&self) -> Option<Range<usize>> {
            self.marked.clone()
        }
        fn attributes_for_character_index(&self, _index: usize) -> Option<CharacterAttributes> {
            None
        }
    }

    fn key(k: Key) -> Event {
        Event::Key { text: String::new(), combo: KeyCombo::new(k) }
    }

    fn session() -> (Server, Mock) {
        let reg = Registration::new("Controller").mode("kana", "Kana");
        let mut server = Server::new(reg).unwrap();
        server.connect(1).unwrap().activate();
        (server, Mock::default())
    }

    #[test]
    fn a_composition_is_shown_as_marked_text_and_replaced_as_it_changes() {
        let (mut server, mut client) = session();
        let c = server.controller_mut(1).unwrap();
        c.set_composition(0, "ka", "か", 3);
        update_composition(c, &mut client);
        assert_eq!(client.text, "か");
        assert_eq!(client.marked_range(), Some(0..3));
        c.set_composition(0, "kak", "かk", 4);
        update_composition(c, &mut client);
        assert_eq!(client.text, "かk", "the marked text is replaced, not appended");
    }

    #[test]
    fn return_commits_into_the_marked_range() {
        let (mut server, mut client) = session();
        let c = server.controller_mut(1).unwrap();
        c.set_composition(0, "ka", "か", 3);
        update_composition(c, &mut client);
        let outcome = c.handle(&key(Key::Return));
        assert!(!apply(&outcome, &mut client));
        assert_eq!(client.text, "か");
        assert_eq!(client.marked_range(), None);
    }

    #[test]
    fn escape_puts_back_the_original_keystrokes() {
        let (mut server, mut client) = session();
        let c = server.controller_mut(1).unwrap();
        c.set_composition(0, "ka", "か", 3);
        update_composition(c, &mut client);
        let outcome = c.handle(&key(Key::Escape));
        assert!(!apply(&outcome, &mut client));
        assert_eq!(client.text, "ka");
    }

    #[test]
    fn events_with_no_composition_are_the_clients() {
        let (mut server, mut client) = session();
        let outcome = server.controller_mut(1).unwrap().handle(&key(Key::Return));
        assert!(apply(&outcome, &mut client));
        assert!(client.calls.is_empty());
    }

    #[test]
    fn an_editor_is_a_client_and_a_composition_is_one_undo_step() {
        use crate::text::TextEditor;
        let mut ed = TextEditor::with_text("ab", 100);
        ed.buf.set_selection(2, 2);
        let reg = Registration::new("Controller").mode("kana", "Kana");
        let mut server = Server::new(reg).unwrap();
        server.connect(1).unwrap().activate();
        let c = server.controller_mut(1).unwrap();
        c.set_composition(2, "ka", "か", 3);
        update_composition(c, &mut ed);
        assert_eq!(ed.buf.text(), "abか");
        assert_eq!(TextInputClient::marked_range(&ed), Some(2..5));
        c.set_composition(2, "kak", "かk", 4);
        update_composition(c, &mut ed);
        assert_eq!(ed.buf.text(), "abかk");
        let outcome = c.handle(&key(Key::Return));
        assert!(!apply(&outcome, &mut ed));
        assert_eq!(ed.buf.text(), "abかk");
        assert_eq!(TextInputClient::marked_range(&ed), None);
        assert!(ed.undo());
        assert_eq!(ed.buf.text(), "ab");
    }

    #[test]
    fn an_editor_commit_over_another_range_replaces_that_range() {
        use crate::text::TextEditor;
        let mut ed = TextEditor::with_text("hello", 100);
        TextInputClient::insert_text(&mut ed, "J", Some(0..1));
        assert_eq!(ed.buf.text(), "Jello");
        assert_eq!(TextInputClient::selected_range(&ed), 1..1);
    }
}
