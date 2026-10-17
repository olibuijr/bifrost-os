//! TipKit for SDK apps: contextual popover tips with display rules and a persistent datastore.
//!
//! The model is `bifrost_ui::tips` (content, rules, the [`Datastore`]); this module re-exports it for apps and adds
//! the two steps an app needs to show a tip: [`present_first`] picks the tip to show and records the display, and
//! [`tip_popover_frame`] places the popover next to the view the tip points at. The shell draws the panel with
//! `widgets::popover`. Apps wire the tip's button to [`Datastore::invalidate`] when the action runs.

use bifrost_ui::geometry::Edge;
use bifrost_ui::split::place_popover;
use bifrost_ui::tips::{Priority, Seconds, TipGroup};
pub use bifrost_ui::tips::{configure, Action, ConfigOption, Datastore, DisplayFrequency, Rule, Tip, TipError, Value};
use bifrost_ui::Rect;

/// Content size of a tip popover, in points: a title, a short message and one button row.
pub const TIP_POPOVER_SIZE_PT: (f32, f32) = (260.0, 96.0);

/// Space between the anchor view and the tip arrow, in points.
pub const TIP_POPOVER_GAP_PT: f32 = 9.0;

/// Presents the first tip of `tips` that may show at `now`, in list order, and records the display in `store`.
/// Returns the index of that tip, or `None` when no tip may show. Only one tip shows per call.
pub fn present_first(store: &mut Datastore, tips: &[&dyn Tip], now: Seconds) -> Result<Option<usize>, TipError> {
    let group = TipGroup::new(Priority::FirstAvailable, tips.to_vec());
    let Some(index) = group.current(store, now) else {
        return Ok(None);
    };
    store.mark_shown(tips[index].id(), now)?;
    Ok(Some(index))
}

/// The rectangle of a tip popover next to `anchor` inside `bounds`, and the edge it ended on. It opens on `preferred`
/// and flips when there is no room.
pub fn tip_popover_frame(anchor: Rect, bounds: Rect, preferred: Edge) -> (Rect, Edge) {
    place_popover(anchor, TIP_POPOVER_SIZE_PT, bounds, preferred, TIP_POPOVER_GAP_PT)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain {
        id: &'static str,
    }

    impl Tip for Plain {
        fn id(&self) -> &str {
            self.id
        }
        fn title(&self) -> &str {
            "Title"
        }
    }

    struct Gated {
        id: &'static str,
    }

    impl Tip for Gated {
        fn id(&self) -> &str {
            self.id
        }
        fn title(&self) -> &str {
            "Gated"
        }
        fn rules(&self) -> Vec<Rule> {
            vec![Rule::EventCount { event: "opened".to_string(), at_least: 3 }]
        }
    }

    fn store() -> Datastore {
        configure(&[ConfigOption::DisplayFrequency(DisplayFrequency::Immediate)]).unwrap()
    }

    #[test]
    fn the_first_eligible_tip_shows_and_is_counted() {
        let mut s = store();
        let a = Gated { id: "a" };
        let b = Plain { id: "b" };
        let tips: [&dyn Tip; 2] = [&a, &b];
        assert_eq!(present_first(&mut s, &tips, 10).unwrap(), Some(1));
        assert_eq!(s.shown_count("b"), 1);
        assert_eq!(s.shown_count("a"), 0);
    }

    #[test]
    fn no_tip_is_presented_when_none_is_eligible() {
        let mut s = store();
        let a = Gated { id: "a" };
        let tips: [&dyn Tip; 1] = [&a];
        assert_eq!(present_first(&mut s, &tips, 10).unwrap(), None);
        s.donate("opened").unwrap();
        s.donate("opened").unwrap();
        s.donate("opened").unwrap();
        assert_eq!(present_first(&mut s, &tips, 10).unwrap(), Some(0));
    }

    #[test]
    fn an_invalidated_tip_is_not_presented_again() {
        let mut s = store();
        let b = Plain { id: "b" };
        let tips: [&dyn Tip; 1] = [&b];
        assert_eq!(present_first(&mut s, &tips, 1).unwrap(), Some(0));
        s.invalidate("b").unwrap();
        assert_eq!(present_first(&mut s, &tips, 2).unwrap(), None);
    }

    #[test]
    fn the_tip_popover_sits_below_its_anchor_when_there_is_room() {
        let anchor = Rect { x: 200.0, y: 60.0, w: 40.0, h: 20.0 };
        let bounds = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        let (r, edge) = tip_popover_frame(anchor, bounds, Edge::Bottom);
        assert_eq!(edge, Edge::Bottom);
        assert_eq!((r.w, r.h), TIP_POPOVER_SIZE_PT);
        assert!(r.y >= anchor.bottom());
    }
}
