//! What a tap on a widget does: open a link, run an app intent, or flip a toggle. Pure model: the action under a
//! point (the topmost hit region wins), and a toggle that shows its new state at once and returns to the old one
//! when the app reports failure. The shell sends the action to the app; nothing here runs an intent.

use crate::widget_link::Link;

/// A rectangle in widget-local points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// True inside the rectangle. The left and top edges are included, the right and bottom edges are not.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open a widget link in its app.
    OpenLink(Link),
    /// Run the app intent with this identifier.
    RunIntent { id: String },
    /// Set the named toggle to the given state.
    SetToggle { id: String, on: bool },
}

/// One tappable region of a widget and the action it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub frame: Rect,
    pub action: Action,
}

/// The action under (x, y). Later regions lie on top, so the last one that contains the point wins.
pub fn hit_test(hits: &[Hit], x: f32, y: f32) -> Option<&Action> {
    hits.iter().rev().find(|h| h.frame.contains(x, y)).map(|h| &h.action)
}

/// A toggle that shows its new state at once. The app's answer then commits it, or puts back the last committed
/// state when the change failed. Taps made while a change is pending are ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    committed: bool,
    shown: bool,
    pending: bool,
}

impl Toggle {
    pub fn new(on: bool) -> Self {
        Toggle { committed: on, shown: on, pending: false }
    }

    /// The state the widget shows.
    pub fn on(&self) -> bool {
        self.shown
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// A tap. Returns the new state to send to the app, or None while an earlier change is still pending.
    pub fn tap(&mut self) -> Option<bool> {
        if self.pending {
            return None;
        }
        let next = !self.shown;
        self.shown = next;
        self.pending = true;
        Some(next)
    }

    /// The app's answer to the last tap. Success commits the shown state. Failure puts back the committed one.
    /// Does nothing when no change is pending.
    pub fn settle(&mut self, ok: bool) {
        if !self.pending {
            return;
        }
        self.pending = false;
        if ok {
            self.committed = self.shown;
        } else {
            self.shown = self.committed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn edges_are_left_edge_inclusive_and_right_edge_exclusive() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(0.0, 0.0));
        assert!(r.contains(9.9_f32, 5.0));
        assert!(!r.contains(10.0, 5.0));
        assert!(!r.contains(5.0, 10.0));
    }

    #[test]
    fn topmost_region_wins_and_outside_is_none() {
        let hits = vec![
            Hit { frame: rect(0.0, 0.0, 100.0, 100.0), action: Action::RunIntent { id: "under".into() } },
            Hit { frame: rect(10.0, 10.0, 20.0, 20.0), action: Action::RunIntent { id: "over".into() } },
        ];
        assert_eq!(hit_test(&hits, 15.0, 15.0), Some(&Action::RunIntent { id: "over".into() }));
        assert_eq!(hit_test(&hits, 50.0, 50.0), Some(&Action::RunIntent { id: "under".into() }));
        assert_eq!(hit_test(&hits, 150.0, 50.0), None);
    }

    #[test]
    fn a_tapped_link_region_yields_its_url() {
        let link = Link::parse("bifrost-app://notes/a").unwrap();
        let hits = vec![Hit { frame: rect(0.0, 0.0, 50.0, 50.0), action: Action::OpenLink(link) }];
        match hit_test(&hits, 1.0, 1.0) {
            Some(Action::OpenLink(l)) => assert_eq!(l.to_url(), "bifrost-app://notes/a"),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn tap_shows_new_state_and_waits_for_the_app() {
        let mut t = Toggle::new(false);
        assert_eq!(t.tap(), Some(true));
        assert!(t.on());
        assert!(t.is_pending());
        assert_eq!(t.tap(), None);
        assert!(t.on());
    }

    #[test]
    fn failed_change_puts_back_the_committed_state() {
        let mut t = Toggle::new(false);
        t.tap();
        t.settle(false);
        assert!(!t.on());
        assert!(!t.is_pending());
        assert_eq!(t.tap(), Some(true));
    }

    #[test]
    fn successful_change_is_committed() {
        let mut t = Toggle::new(true);
        assert_eq!(t.tap(), Some(false));
        t.settle(true);
        assert!(!t.on());
        assert_eq!(t.tap(), Some(true));
    }

    #[test]
    fn settle_without_a_pending_change_does_nothing() {
        let mut t = Toggle::new(true);
        t.settle(false);
        assert!(t.on());
        assert!(!t.is_pending());
    }
}
