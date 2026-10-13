//! Composing input methods: the provider side of text input.
//!
//! The controller mirrors the reference input-method kit: `InputController` carries the server-input calls
//! (`input_text`, `handle`, `commit_composition`, `composed_string`, `cancel_composition`) and talks to the
//! app's text field through `TextClient` (`set_marked_text`, `insert_text`). A `Composer` is the small state
//! machine behind one method: `hangul` (Korean 2-set) and `telex` (Vietnamese Telex and VNI).
//!
//! Ranges count characters, not UTF-16 units. Not wired into `kb_input.rs` yet: that is a follow-up, the
//! shell would look a controller up with `new_controller(source_id)` and route keys to it.

pub mod hangul;
pub mod telex;

/// A range in the text the client holds (`location`, `length`), in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    pub location: usize,
    pub length: usize,
}

/// The text field a controller drives (the text-input protocol of the client app).
pub trait TextClient {
    /// Show `text` as marked (uncommitted) text. `selection` is inside the marked string; an empty string clears the marking.
    fn set_marked_text(&mut self, text: &str, selection: TextRange, replacement: Option<TextRange>);
    /// Commit `text`; this also ends any marking.
    fn insert_text(&mut self, text: &str, replacement: Option<TextRange>);
}

/// A key event the shell forwards to the method (`handle(_:client:)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Return,
    Tab,
    Escape,
}

/// The state machine behind one method.
pub trait Composer {
    /// Take one typed character. `Some(committed)` means it was consumed and `committed` is text that is
    /// final now (often empty); `None` means it is not a composing key and the composition must end.
    fn feed(&mut self, ch: char) -> Option<String>;
    /// Undo the last step. False when nothing is being composed.
    fn backspace(&mut self) -> bool;
    /// The marked string.
    fn composed(&self) -> String;
    /// Drop the composition.
    fn reset(&mut self);
}

/// One session of an input method (`IMKInputController`).
pub trait InputController {
    /// `inputText(_:client:)`: true when the method took the text, false when the client should handle it.
    fn input_text(&mut self, string: &str, client: &mut dyn TextClient) -> bool;
    /// `handle(_:client:)`: true when the event was consumed.
    fn handle(&mut self, key: Key, client: &mut dyn TextClient) -> bool;
    /// `commitComposition(_:)`: insert the marked text and clear the marking.
    fn commit_composition(&mut self, client: &mut dyn TextClient);
    /// `composedString(_:)`: the current marked text.
    fn composed_string(&self) -> String;
    /// `cancelComposition()`: drop the marked text without inserting it.
    fn cancel_composition(&mut self, client: &mut dyn TextClient);
}

/// An `InputController` over any `Composer`.
pub struct ComposingController<C: Composer> {
    composer: C,
}

impl<C: Composer> ComposingController<C> {
    pub fn new(composer: C) -> Self {
        Self { composer }
    }

    fn composing(&self) -> bool {
        !self.composer.composed().is_empty()
    }

    fn refresh(&self, client: &mut dyn TextClient) {
        let s = self.composer.composed();
        let end = TextRange { location: s.chars().count(), length: 0 };
        client.set_marked_text(&s, end, None);
    }
}

impl<C: Composer> InputController for ComposingController<C> {
    fn input_text(&mut self, string: &str, client: &mut dyn TextClient) -> bool {
        let mut handled = false;
        for ch in string.chars() {
            match self.composer.feed(ch) {
                Some(done) => {
                    handled = true;
                    if !done.is_empty() {
                        client.insert_text(&done, None);
                    }
                }
                None if self.composing() => {
                    handled = true;
                    self.commit_composition(client);
                    client.insert_text(&ch.to_string(), None);
                }
                None => {}
            }
        }
        if handled && self.composing() {
            self.refresh(client);
        }
        handled
    }

    fn handle(&mut self, key: Key, client: &mut dyn TextClient) -> bool {
        match key {
            Key::Char(c) => self.input_text(&c.to_string(), client),
            Key::Backspace => {
                if !self.composer.backspace() {
                    return false;
                }
                self.refresh(client);
                true
            }
            Key::Escape if self.composing() => {
                self.cancel_composition(client);
                true
            }
            Key::Return | Key::Tab if self.composing() => {
                self.commit_composition(client);
                key == Key::Return
            }
            _ => false,
        }
    }

    fn commit_composition(&mut self, client: &mut dyn TextClient) {
        let s = self.composer.composed();
        self.composer.reset();
        if !s.is_empty() {
            client.insert_text(&s, None);
        }
    }

    fn composed_string(&self) -> String {
        self.composer.composed()
    }

    fn cancel_composition(&mut self, client: &mut dyn TextClient) {
        self.composer.reset();
        client.set_marked_text("", TextRange { location: 0, length: 0 }, None);
    }
}

/// A selectable input source (`TISInputSource`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSource {
    pub id: &'static str,
    pub localized_name: &'static str,
    pub language: &'static str,
}

pub const KOREAN_2SET: &str = "bifrost.inputmethod.Korean.2SetKorean";
pub const VIETNAMESE_TELEX: &str = "bifrost.inputmethod.VietnameseIM.VietnameseTelex";
pub const VIETNAMESE_VNI: &str = "bifrost.inputmethod.VietnameseIM.VietnameseVNI";

const SOURCES: [InputSource; 3] = [
    InputSource { id: KOREAN_2SET, localized_name: "2-Set Korean", language: "ko" },
    InputSource { id: VIETNAMESE_TELEX, localized_name: "Vietnamese Telex", language: "vi" },
    InputSource { id: VIETNAMESE_VNI, localized_name: "Vietnamese VNI", language: "vi" },
];

/// The composing sources this module provides.
pub fn sources() -> &'static [InputSource] {
    &SOURCES
}

/// A fresh controller for a source id, or None for an id this module does not provide.
pub fn new_controller(id: &str) -> Option<Box<dyn InputController>> {
    match id {
        KOREAN_2SET => Some(Box::new(ComposingController::new(hangul::HangulComposer::default()))),
        VIETNAMESE_TELEX => Some(Box::new(ComposingController::new(telex::VietnameseComposer::new(telex::Scheme::Telex)))),
        VIETNAMESE_VNI => Some(Box::new(ComposingController::new(telex::VietnameseComposer::new(telex::Scheme::Vni)))),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Records what the controller sent; `text()` is committed plus marked, the way the field shows it.
    #[derive(Default)]
    pub struct RecordingClient {
        pub committed: String,
        pub marked: String,
    }

    impl RecordingClient {
        pub fn text(&self) -> String {
            format!("{}{}", self.committed, self.marked)
        }
    }

    impl TextClient for RecordingClient {
        fn set_marked_text(&mut self, text: &str, _selection: TextRange, _replacement: Option<TextRange>) {
            self.marked = text.to_string();
        }
        fn insert_text(&mut self, text: &str, _replacement: Option<TextRange>) {
            self.committed.push_str(text);
            self.marked.clear();
        }
    }

    /// Type `keys` into a controller for `id` and return the client.
    pub fn type_keys(id: &str, keys: &str) -> (Box<dyn InputController>, RecordingClient) {
        let mut ctl = new_controller(id).expect("source");
        let mut client = RecordingClient::default();
        for ch in keys.chars() {
            ctl.handle(Key::Char(ch), &mut client);
        }
        (ctl, client)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn sources_are_listed_and_buildable() {
        assert_eq!(sources().len(), 3);
        for s in sources() {
            assert!(new_controller(s.id).is_some(), "{}", s.id);
        }
        assert!(new_controller("bifrost.inputmethod.Nope").is_none());
    }

    #[test]
    fn commit_inserts_marked_text_and_clears_it() {
        let (mut ctl, mut c) = type_keys(KOREAN_2SET, "gks");
        assert_eq!(ctl.composed_string(), "한");
        assert_eq!(c.marked, "한");
        ctl.commit_composition(&mut c);
        assert_eq!((c.committed.as_str(), c.marked.as_str()), ("한", ""));
        assert_eq!(ctl.composed_string(), "");
    }

    #[test]
    fn cancel_drops_marked_text() {
        let (mut ctl, mut c) = type_keys(VIETNAMESE_TELEX, "vieet");
        assert_eq!(c.marked, "viêt");
        ctl.cancel_composition(&mut c);
        assert_eq!(c.text(), "");
        assert_eq!(ctl.composed_string(), "");
    }

    #[test]
    fn input_text_reports_unhandled_when_idle() {
        let mut ctl = new_controller(KOREAN_2SET).unwrap();
        let mut c = RecordingClient::default();
        assert!(!ctl.input_text("1", &mut c));
        assert!(!ctl.handle(Key::Backspace, &mut c));
        assert!(!ctl.handle(Key::Return, &mut c));
        assert_eq!(c.text(), "");
    }

    #[test]
    fn escape_cancels_and_return_commits() {
        let (mut ctl, mut c) = type_keys(KOREAN_2SET, "gk");
        assert!(ctl.handle(Key::Escape, &mut c));
        assert_eq!(c.text(), "");
        ctl.handle(Key::Char('g'), &mut c);
        assert!(ctl.handle(Key::Return, &mut c));
        assert_eq!(c.committed, "ㅎ");
    }
}
