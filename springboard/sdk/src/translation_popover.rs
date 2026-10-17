//! System translation popover for SDK apps (`translationPresentation`). The app selects text, presents the popover
//! with the arrow edge it wants, and the popover runs the translation through the app's `Translator`, checking the
//! language pair against the installed languages first. The app reads the result from [`TranslationPresenter::replacement`]
//! and writes it into its own document; nothing is written for it.
//!
//! The state lives in `bifrost_ui::translation_popover::Popover`. This module adds the engine call and the placement
//! of the panel next to the selection (`bifrost_ui::split::place_popover`), which the shell draws with
//! `widgets::popover`.

use crate::translation::{TranslationError, Translator};
use bifrost_ui::geometry::Edge;
use bifrost_ui::split::place_popover;
use bifrost_ui::translation::{language_code, LanguageAvailability};
pub use bifrost_ui::translation_popover::{ArrowEdge, Phase, Popover};
use bifrost_ui::Rect;

/// Content size of the translation panel, in points. The text wraps inside it.
pub const TRANSLATION_POPOVER_SIZE_PT: (f32, f32) = (280.0, 120.0);

/// Space between the anchor and the panel arrow, in points (matches `widgets::POPOVER_ARROW_PT`).
pub const TRANSLATION_POPOVER_GAP_PT: f32 = 9.0;

/// The popover of one app window: a [`Popover`] plus the engine call that fills it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranslationPresenter {
    popover: Popover,
}

impl TranslationPresenter {
    /// A presenter whose popover is hidden.
    pub fn new() -> Self {
        Self::default()
    }

    /// The popover state, for drawing.
    pub fn popover(&self) -> &Popover {
        &self.popover
    }

    /// Presents the popover for `text` from `source` to `target`, then runs the translation through `engine` and
    /// records its answer. A refused pair leaves the popover hidden and returns the reason. The engine receives
    /// normalised language codes, as [`TranslationSession`](crate::translation::TranslationSession) does.
    pub fn present(
        &mut self,
        text: &str,
        source: &str,
        target: &str,
        edge: ArrowEdge,
        availability: &LanguageAvailability,
        engine: &dyn Translator,
    ) -> Result<(), TranslationError> {
        self.popover.present(text, source, target, edge, availability)?;
        let result = engine.translate(&language_code(source), &language_code(target), text);
        self.popover.finish(result);
        Ok(())
    }

    /// The translated text the app may write in place of the selection, once the translation finished.
    pub fn replacement(&self) -> Option<&str> {
        self.popover.replace()
    }

    /// The rectangle the panel occupies next to `anchor` inside `bounds`, and the edge it ended on. `None` when the
    /// popover is hidden. The panel opens on the edge the popover was presented with and flips when there is no room.
    pub fn placement(&self, anchor: Rect, bounds: Rect) -> Option<(Rect, Edge)> {
        if !self.popover.is_presented() {
            return None;
        }
        let preferred = match self.popover.arrow_edge() {
            ArrowEdge::Top => Edge::Top,
            ArrowEdge::Bottom => Edge::Bottom,
            ArrowEdge::Leading => Edge::Left,
            ArrowEdge::Trailing => Edge::Right,
        };
        Some(place_popover(anchor, TRANSLATION_POPOVER_SIZE_PT, bounds, preferred, TRANSLATION_POPOVER_GAP_PT))
    }

    /// Hides the popover and drops its text and result.
    pub fn dismiss(&mut self) {
        self.popover.dismiss();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Upper;

    impl Translator for Upper {
        fn translate(&self, source: &str, target: &str, text: &str) -> Result<String, TranslationError> {
            assert_eq!((source, target), ("en", "is"));
            Ok(text.to_uppercase())
        }
    }

    struct Broken;

    impl Translator for Broken {
        fn translate(&self, _: &str, _: &str, _: &str) -> Result<String, TranslationError> {
            Err(TranslationError::InternalError)
        }
    }

    fn installed() -> LanguageAvailability {
        let mut a = LanguageAvailability::new(&["en", "is", "de"]);
        a.install("en", "is");
        a
    }

    fn screen() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }
    }

    #[test]
    fn a_hidden_presenter_has_no_placement_and_no_replacement() {
        let p = TranslationPresenter::new();
        assert_eq!(p.placement(Rect { x: 100.0, y: 100.0, w: 50.0, h: 20.0 }, screen()), None);
        assert_eq!(p.replacement(), None);
    }

    #[test]
    fn presenting_runs_the_engine_and_offers_the_replacement() {
        let mut p = TranslationPresenter::new();
        p.present("hello", "en_US", "is", ArrowEdge::Bottom, &installed(), &Upper).unwrap();
        assert_eq!(p.replacement(), Some("HELLO"));
        assert_eq!(p.popover().phase(), &Phase::Shown("HELLO".to_string()));
    }

    #[test]
    fn an_engine_failure_is_shown_and_offers_no_replacement() {
        let mut p = TranslationPresenter::new();
        p.present("hello", "en", "is", ArrowEdge::Bottom, &installed(), &Broken).unwrap();
        assert_eq!(p.replacement(), None);
        assert_eq!(p.popover().phase(), &Phase::Failed(TranslationError::InternalError));
    }

    #[test]
    fn a_refused_pair_never_calls_the_engine() {
        let mut p = TranslationPresenter::new();
        let err = p.present("hello", "en", "de", ArrowEdge::Bottom, &installed(), &Broken);
        assert_eq!(err, Err(TranslationError::NotInstalled));
        assert!(!p.popover().is_presented());
    }

    #[test]
    fn the_panel_sits_below_a_selection_near_the_top_of_the_screen() {
        let mut p = TranslationPresenter::new();
        p.present("hello", "en", "is", ArrowEdge::Bottom, &installed(), &Upper).unwrap();
        let anchor = Rect { x: 300.0, y: 40.0, w: 80.0, h: 20.0 };
        let (r, edge) = p.placement(anchor, screen()).unwrap();
        assert_eq!(edge, Edge::Bottom);
        assert_eq!((r.w, r.h), TRANSLATION_POPOVER_SIZE_PT);
        assert!(r.y >= anchor.bottom());
    }

    #[test]
    fn dismiss_hides_the_panel_and_drops_the_result() {
        let mut p = TranslationPresenter::new();
        p.present("hello", "en", "is", ArrowEdge::Top, &installed(), &Upper).unwrap();
        p.dismiss();
        assert_eq!(p.replacement(), None);
        assert_eq!(p.placement(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, screen()), None);
    }
}
