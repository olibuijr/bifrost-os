//! Large content viewer (the reference platform's large content viewer interaction): a press that is held still for a moment presents the item
//! in enlarged form, for people who use large text sizes. This is pure state. The SDK wrapper feeds it pointer events
//! and frame times and draws what it reports; the item's title and image are the app's.

/// Seconds a press must be held still before the viewer appears.
pub const HOLD_S: f64 = 0.5;
/// Points the finger may travel before the hold counts as a drag and is cancelled.
pub const SLOP_PT: f32 = 10.0;

/// What the large content viewer shows for an item (the reference platform's large content viewer item).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LargeContentItem {
    pub title: String,
    /// the name of the symbol the app draws, if any
    pub image: Option<String>,
    /// `showsLargeContentViewer`: whether the item takes part at all
    pub shows: bool,
}

impl LargeContentItem {
    pub fn new(title: &str) -> Self {
        LargeContentItem { title: title.into(), image: None, shows: true }
    }
}

/// Where a press is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    /// held since `started` (seconds) at the point `(x, y)`
    Pressing { started: f64, x: f32, y: f32 },
    Showing,
}

/// A change the owner must act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Present,
    Dismiss,
}

/// The hold-to-enlarge state machine. Disabled, it never leaves `Idle`.
#[derive(Clone, Debug, PartialEq)]
pub struct LargeContentInteraction {
    /// `isEnabled`: the viewer is in use on this device (the owner decides from the text size)
    pub enabled: bool,
    pub hold_s: f64,
    pub slop: f32,
    phase: Phase,
}

impl LargeContentInteraction {
    pub fn new(enabled: bool) -> Self {
        LargeContentInteraction { enabled, hold_s: HOLD_S, slop: SLOP_PT, phase: Phase::Idle }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn is_showing(&self) -> bool {
        self.phase == Phase::Showing
    }

    /// True while a press waits for its hold to complete, so the owner asks for frame times.
    pub fn wants_tick(&self) -> bool {
        matches!(self.phase, Phase::Pressing { .. })
    }

    /// A press began at `now` (seconds) at `(x, y)`.
    pub fn press(&mut self, now: f64, x: f32, y: f32) {
        self.phase = if self.enabled { Phase::Pressing { started: now, x, y } } else { Phase::Idle };
    }

    /// The pointer moved to `(x, y)`: a move past the slop cancels a pending hold. A shown viewer is not affected.
    pub fn moved(&mut self, x: f32, y: f32) {
        if let Phase::Pressing { x: x0, y: y0, .. } = self.phase {
            if (x - x0).hypot(y - y0) > self.slop {
                self.phase = Phase::Idle;
            }
        }
    }

    /// Frame time `now` (seconds). A press held for `hold_s` presents the viewer.
    pub fn tick(&mut self, now: f64) -> Option<Change> {
        if let Phase::Pressing { started, .. } = self.phase {
            if now - started >= self.hold_s {
                self.phase = Phase::Showing;
                return Some(Change::Present);
            }
        }
        None
    }

    /// The finger lifted, or the press was taken away. Returns `Dismiss` when the viewer was showing.
    pub fn end(&mut self) -> Option<Change> {
        let was_showing = self.is_showing();
        self.phase = Phase::Idle;
        was_showing.then_some(Change::Dismiss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_hold_presents_the_viewer_once_the_hold_time_has_passed() {
        let mut i = LargeContentInteraction::new(true);
        i.press(1.0, 5.0, 5.0);
        assert!(i.wants_tick());
        assert_eq!(i.tick(1.2), None);
        assert_eq!(i.tick(1.0 + HOLD_S), Some(Change::Present));
        assert!(i.is_showing());
        assert!(!i.wants_tick());
    }

    #[test]
    fn lifting_the_finger_dismisses_a_shown_viewer() {
        let mut i = LargeContentInteraction::new(true);
        i.press(0.0, 0.0, 0.0);
        i.tick(10.0);
        assert_eq!(i.end(), Some(Change::Dismiss));
        assert_eq!(i.phase(), Phase::Idle);
    }

    #[test]
    fn a_release_before_the_hold_presents_nothing() {
        let mut i = LargeContentInteraction::new(true);
        i.press(0.0, 0.0, 0.0);
        assert_eq!(i.end(), None);
        assert_eq!(i.tick(5.0), None);
        assert_eq!(i.phase(), Phase::Idle);
    }

    #[test]
    fn a_move_within_the_slop_keeps_the_hold_and_a_longer_one_cancels_it() {
        let mut i = LargeContentInteraction::new(true);
        i.press(0.0, 0.0, 0.0);
        i.moved(3.0, 4.0);
        assert!(i.wants_tick());
        i.moved(20.0, 0.0);
        assert_eq!(i.phase(), Phase::Idle);
        assert_eq!(i.tick(10.0), None);
    }

    #[test]
    fn a_disabled_interaction_never_presses() {
        let mut i = LargeContentInteraction::new(false);
        i.press(0.0, 0.0, 0.0);
        assert!(!i.wants_tick());
        assert_eq!(i.tick(5.0), None);
    }

    #[test]
    fn moving_a_shown_viewer_leaves_it_shown() {
        let mut i = LargeContentInteraction::new(true);
        i.press(0.0, 0.0, 0.0);
        i.tick(1.0);
        i.moved(100.0, 100.0);
        assert!(i.is_showing());
    }
}
