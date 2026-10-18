//! The writing assistant's proofreading panel: what an app's Proofread command holds while the person goes through
//! the suggestions. It opens a [`Session`] (so a view that turned the assistant off gets nothing), keeps the findings
//! with the sentence that explains each, and answers Accept, Skip and Accept All. Accepting hands back the change as a
//! byte-range replacement for the app to make in its own text (so it keeps its undo and styles) and moves the other
//! suggestions with the change in length. The panel draws nothing: an app lists [`Panel::describe`] lines in its own
//! menu or sheet.
//!
//! Everything runs on the device on the text it is given; nothing is kept after the panel is dropped.

use crate::proofread::{proofread, Options};
use crate::writing_assist::{context_range, Behavior, ContextScope, Session, SessionError, Suggestion, Task};
use std::ops::Range;

/// Letters that mark a text as not English (the Icelandic ones). The English rules of the proofreader run only on
/// text without them.
const NOT_ENGLISH: &str = "áéíóúýþæöðÁÉÍÓÚÝÞÆÖÐ";

/// True when `text` has none of the letters that only other languages use. A crude test: an app that knows its text's
/// language passes that instead.
pub fn english_by_letters(text: &str) -> bool {
    !text.chars().any(|c| NOT_ENGLISH.contains(c))
}

/// The suggestions of one proofreading task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    suggestions: Vec<Suggestion>,
    messages: Vec<&'static str>,
    context: Range<usize>,
    at: usize,
}

impl Panel {
    /// Proofreads `text`: the byte range `selection` when it is not empty, else the whole text. Errors when the
    /// assistant is off for the view.
    pub fn open(text: &str, selection: Range<usize>, options: Options) -> Result<Panel, SessionError> {
        let scope = if selection.start < selection.end { ContextScope::UserSelection } else { ContextScope::FullDocument };
        let context = context_range(text, scope, selection, 0..text.len());
        let findings = proofread(text, context.clone(), options);
        let mut session = Session::new(Behavior::Default.resolve(true));
        session.begin(Task::Proofread, context)?;
        session.offer(findings.iter().map(|f| f.suggestion.clone()).collect())?;
        Ok(Panel { suggestions: session.pending().to_vec(), messages: findings.iter().map(|f| f.message).collect(), context: session.context(), at: 0 })
    }

    /// A session holding what is left, to run an accept or a reject through the session's checks.
    fn session(&self) -> Result<Session, SessionError> {
        let mut session = Session::new(Behavior::Default.resolve(true));
        session.begin(Task::Proofread, self.context.clone())?;
        session.offer(self.suggestions.clone())?;
        Ok(session)
    }

    pub fn len(&self) -> usize {
        self.suggestions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.suggestions.is_empty()
    }

    /// The suggestions left, in text order.
    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    /// The index of the suggestion in view.
    pub fn at(&self) -> usize {
        self.at
    }

    /// The suggestion in view, with the sentence that explains it.
    pub fn current(&self) -> Option<(&Suggestion, &'static str)> {
        Some((self.suggestions.get(self.at)?, *self.messages.get(self.at)?))
    }

    /// Moves to the next suggestion, wrapping at the end.
    pub fn next(&mut self) {
        if !self.suggestions.is_empty() {
            self.at = (self.at + 1) % self.suggestions.len();
        }
    }

    /// One line for suggestion `index` of `text`: the sentence that explains it, then the change as Remove, Add or
    /// Change.
    pub fn describe(&self, text: &str, index: usize) -> Option<String> {
        let s = self.suggestions.get(index)?;
        let message = *self.messages.get(index)?;
        let show = |part: &str| if part.trim().is_empty() { "a space".to_string() } else { format!("\"{}\"", part.trim()) };
        let before = text.get(s.range.clone()).unwrap_or("");
        Some(match (before.is_empty(), s.replacement.is_empty()) {
            (true, true) => message.to_string(),
            (true, false) => format!("{message} Add {}.", show(&s.replacement)),
            (false, true) => format!("{message} Remove {}.", show(before)),
            (false, false) => format!("{message} Change {} to {}.", show(before), show(&s.replacement)),
        })
    }

    /// A short line for suggestion `index` of `text`, for a menu row: `Remove "it"`, `Add a space` or
    /// `Change "teh" to "the"`.
    pub fn brief(&self, text: &str, index: usize) -> Option<String> {
        let s = self.suggestions.get(index)?;
        let show = |part: &str| if part.trim().is_empty() { "a space".to_string() } else { format!("\"{}\"", part.trim()) };
        let before = text.get(s.range.clone()).unwrap_or("");
        Some(match (before.is_empty(), s.replacement.is_empty()) {
            (true, true) => "Change".to_string(),
            (true, false) => format!("Add {}", show(&s.replacement)),
            (false, true) => format!("Remove {}", show(before)),
            (false, false) => format!("Change {} to {}", show(before), show(&s.replacement)),
        })
    }

    /// Accepts the suggestion in view. Returns it (a byte range of the text as it was, and the replacement) and the text
    /// with it applied. The suggestions that remain are moved to the new text.
    pub fn accept(&mut self, text: &str) -> Result<(Suggestion, String), SessionError> {
        self.accept_at(text, self.at)
    }

    /// Accepts suggestion `index` (see [`Panel::accept`]).
    pub fn accept_at(&mut self, text: &str, index: usize) -> Result<(Suggestion, String), SessionError> {
        let chosen = self.suggestions.get(index).cloned().ok_or(SessionError::NoSuggestion(index))?;
        let mut session = self.session()?;
        let new_text = session.accept(text, index)?;
        // the session dropped the accepted suggestion and any that overlapped it: drop the same sentences
        let kept: Vec<bool> = self.suggestions.iter().enumerate().map(|(i, p)| i != index && !(p.range.start < chosen.range.end && chosen.range.start < p.range.end)).collect();
        self.messages = self.messages.iter().zip(&kept).filter(|(_, k)| **k).map(|(m, _)| *m).collect();
        self.suggestions = session.pending().to_vec();
        self.context = session.context();
        self.at = index.min(self.suggestions.len().saturating_sub(1));
        Ok((chosen, new_text))
    }

    /// Skips the suggestion in view; the text does not change.
    pub fn skip(&mut self) -> Result<(), SessionError> {
        self.skip_at(self.at)
    }

    /// Skips suggestion `index`.
    pub fn skip_at(&mut self, index: usize) -> Result<(), SessionError> {
        let mut session = self.session()?;
        session.reject(index)?;
        self.messages.remove(index);
        self.suggestions = session.pending().to_vec();
        self.at = index.min(self.suggestions.len().saturating_sub(1));
        Ok(())
    }

    /// Accepts every suggestion left. Returns them in the order to apply: each is a byte range of the text after the
    /// ones before it were applied. The panel is empty afterwards.
    pub fn accept_all(&mut self, text: &str) -> Result<Vec<Suggestion>, SessionError> {
        let mut current = text.to_string();
        let mut done = Vec::new();
        while !self.suggestions.is_empty() {
            let (chosen, new_text) = self.accept_at(&current, 0)?;
            current = new_text;
            done.push(chosen);
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing_assist::apply;

    const ANY: Options = Options { english: false };
    const EN: Options = Options { english: true };

    #[test]
    fn the_panel_lists_the_findings_and_describes_each() {
        let text = "Say it it again, red,green";
        let panel = Panel::open(text, 0..0, ANY).unwrap();
        assert_eq!(panel.len(), 2);
        let first = panel.describe(text, 0).unwrap();
        assert!(first.ends_with("Remove \"it\"."), "{first}");
        let second = panel.describe(text, 1).unwrap();
        assert!(second.ends_with("Add a space."), "{second}");
        assert_eq!(panel.brief(text, 0).as_deref(), Some("Remove \"it\""));
        assert_eq!(panel.brief(text, 1).as_deref(), Some("Add a space"));
        assert!(panel.describe(text, 2).is_none() && panel.brief(text, 2).is_none());
    }

    #[test]
    fn accepting_one_moves_the_others_with_the_change() {
        let text = "Say it it again, red,green";
        let mut panel = Panel::open(text, 0..0, ANY).unwrap();
        let (_, once) = panel.accept(text).unwrap();
        assert_eq!(once, "Say it again, red,green");
        assert_eq!(panel.len(), 1);
        let (second, twice) = panel.accept(&once).unwrap();
        assert_eq!(twice, "Say it again, red, green");
        assert_eq!(apply(&once, &[second]).unwrap(), twice, "the range fits the text it was handed back for");
        assert!(panel.is_empty());
    }

    #[test]
    fn skip_drops_one_and_accept_all_applies_the_rest_in_order() {
        let text = "the the  cat  sat";
        let mut panel = Panel::open(text, 0..0, ANY).unwrap();
        let n = panel.len();
        assert!(n >= 2);
        panel.skip().unwrap();
        assert_eq!(panel.len(), n - 1);
        let steps = panel.accept_all(text).unwrap();
        assert_eq!(steps.len(), n - 1);
        assert!(panel.is_empty());
        let mut cur = text.to_string();
        for s in steps {
            cur = apply(&cur, &[s]).unwrap();
        }
        assert_ne!(cur, text);
    }

    #[test]
    fn a_selection_limits_what_is_read() {
        let text = "ok ok. fine fine";
        let panel = Panel::open(text, 7..text.len(), ANY).unwrap();
        assert_eq!(panel.len(), 1);
        assert_eq!(Panel::open(text, 0..0, ANY).unwrap().len(), 2);
    }

    #[test]
    fn the_english_rules_follow_the_flag_and_the_letters() {
        assert!(english_by_letters("it ends. then more"));
        assert!(!english_by_letters("Ég sá hana"));
        assert_eq!(Panel::open("it ends. then more", 0..0, EN).unwrap().len(), 1);
        assert_eq!(Panel::open("it ends. then more", 0..0, ANY).unwrap().len(), 0);
    }
}
