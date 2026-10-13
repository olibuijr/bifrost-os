//! System translation popover as a pure model: the state an app shows while it translates a selection in place.
//!
//! The app asks to present the popover for a text, with the edge the arrow points to. The model checks the language
//! pair against [`LanguageAvailability`] before it accepts the text, so the popover never starts a translation that
//! the pair cannot run. The translation engine reports its result through [`Popover::finish`]. A replacement is
//! offered only for a finished translation, and [`Popover::replace`] hands the translated text back to the app, which
//! writes it into its own document. Nothing is drawn here and no engine runs here.

use crate::translation::{LanguageAvailability, TranslationError};

/// The edge of the anchor the popover arrow points from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowEdge {
    Top,
    Bottom,
    Leading,
    Trailing,
}

/// Where the popover is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not presented.
    Hidden,
    /// Presented, and the engine has not reported a result yet.
    Translating,
    /// Presented with a finished translation.
    Shown(String),
    /// Presented with the error the engine or the language check reported.
    Failed(TranslationError),
}

/// The popover for one selection. `presented` mirrors `isPresented` in the framework.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popover {
    presented: bool,
    text: String,
    source: String,
    target: String,
    edge: ArrowEdge,
    phase: Phase,
}

impl Default for Popover {
    fn default() -> Self {
        Self { presented: false, text: String::new(), source: String::new(), target: String::new(), edge: ArrowEdge::Bottom, phase: Phase::Hidden }
    }
}

impl Popover {
    /// A popover that is not presented.
    pub fn new() -> Self {
        Self::default()
    }

    /// `isPresented`: true from a successful `present` until `dismiss`.
    pub fn is_presented(&self) -> bool {
        self.presented
    }

    /// The text the popover translates.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The arrow edge the popover was presented with.
    pub fn arrow_edge(&self) -> ArrowEdge {
        self.edge
    }

    /// The current phase.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Presents the popover for `text`, translating from `source` to `target`. A blank text is refused with
    /// `NothingToTranslate`. A pair that is not installed is refused with the error the session check reports, and
    /// the popover stays hidden. A presented popover that is asked to present again moves to the new text.
    pub fn present(
        &mut self,
        text: &str,
        source: &str,
        target: &str,
        edge: ArrowEdge,
        availability: &LanguageAvailability,
    ) -> Result<(), TranslationError> {
        if text.trim().is_empty() {
            return Err(TranslationError::NothingToTranslate);
        }
        availability.check_session(source, target)?;
        self.presented = true;
        self.text = text.to_string();
        self.source = source.to_string();
        self.target = target.to_string();
        self.edge = edge;
        self.phase = Phase::Translating;
        Ok(())
    }

    /// Records the engine's answer for the presented text. Ignored when the popover is not presented or no translation
    /// is running, so a late answer after a dismissal is dropped.
    pub fn finish(&mut self, result: Result<String, TranslationError>) {
        if !self.presented || self.phase != Phase::Translating {
            return;
        }
        self.phase = match result {
            Ok(translated) => Phase::Shown(translated),
            Err(e) => Phase::Failed(e),
        };
    }

    /// The translated text the app may write back in place of the selection. Only a shown translation offers one.
    pub fn replace(&self) -> Option<&str> {
        match &self.phase {
            Phase::Shown(translated) if self.presented => Some(translated.as_str()),
            _ => None,
        }
    }

    /// The language pair the presented text is translated with, as `(source, target)`.
    pub fn languages(&self) -> Option<(&str, &str)> {
        self.presented.then_some((self.source.as_str(), self.target.as_str()))
    }

    /// Hides the popover and drops its text and result.
    pub fn dismiss(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> LanguageAvailability {
        let mut a = LanguageAvailability::new(&["en", "is", "de"]);
        a.install("en", "is");
        a
    }

    #[test]
    fn a_new_popover_is_hidden_with_no_text() {
        let p = Popover::new();
        assert!(!p.is_presented());
        assert_eq!(p.phase(), &Phase::Hidden);
        assert_eq!(p.replace(), None);
        assert_eq!(p.languages(), None);
    }

    #[test]
    fn present_then_finish_shows_a_translation_that_can_replace_the_selection() {
        let mut p = Popover::new();
        p.present("Hello", "en", "is", ArrowEdge::Top, &installed()).unwrap();
        assert!(p.is_presented());
        assert_eq!(p.phase(), &Phase::Translating);
        assert_eq!(p.replace(), None, "no replacement while the engine is still running");
        p.finish(Ok("Halló".to_string()));
        assert_eq!(p.phase(), &Phase::Shown("Halló".to_string()));
        assert_eq!(p.replace(), Some("Halló"));
        assert_eq!(p.arrow_edge(), ArrowEdge::Top);
        assert_eq!(p.languages(), Some(("en", "is")));
    }

    #[test]
    fn blank_text_is_refused_and_leaves_the_popover_hidden() {
        let mut p = Popover::new();
        assert_eq!(p.present("   ", "en", "is", ArrowEdge::Bottom, &installed()), Err(TranslationError::NothingToTranslate));
        assert!(!p.is_presented());
    }

    #[test]
    fn a_pair_that_is_not_installed_is_refused_before_presenting() {
        let mut p = Popover::new();
        let err = p.present("Hello", "en", "de", ArrowEdge::Bottom, &installed());
        assert_eq!(err, Err(TranslationError::NotInstalled));
        assert!(!p.is_presented());
    }

    #[test]
    fn an_unsupported_language_is_reported_with_its_framework_error() {
        let mut p = Popover::new();
        let err = p.present("Hello", "xx", "is", ArrowEdge::Bottom, &installed());
        assert_eq!(err, Err(TranslationError::UnsupportedSourceLanguage));
    }

    #[test]
    fn an_engine_error_is_shown_and_offers_no_replacement() {
        let mut p = Popover::new();
        p.present("Hello", "en", "is", ArrowEdge::Leading, &installed()).unwrap();
        p.finish(Err(TranslationError::InternalError));
        assert_eq!(p.phase(), &Phase::Failed(TranslationError::InternalError));
        assert_eq!(p.replace(), None);
        assert!(p.is_presented());
    }

    #[test]
    fn a_late_result_after_dismiss_is_dropped() {
        let mut p = Popover::new();
        p.present("Hello", "en", "is", ArrowEdge::Bottom, &installed()).unwrap();
        p.dismiss();
        p.finish(Ok("Halló".to_string()));
        assert!(!p.is_presented());
        assert_eq!(p.phase(), &Phase::Hidden);
        assert_eq!(p.text(), "");
    }

    #[test]
    fn presenting_again_moves_to_the_new_text_and_waits_for_its_result() {
        let mut p = Popover::new();
        p.present("Hello", "en", "is", ArrowEdge::Bottom, &installed()).unwrap();
        p.finish(Ok("Halló".to_string()));
        p.present("Bye", "en", "is", ArrowEdge::Trailing, &installed()).unwrap();
        assert_eq!(p.text(), "Bye");
        assert_eq!(p.phase(), &Phase::Translating);
        assert_eq!(p.replace(), None);
    }
}
