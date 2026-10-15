//! Two-finger trackpad gestures as one event stream: magnification and rotation (the events behind pointer magnify and
//! rotate). The compositor feeds each frame with both finger positions and then the lift. The pinch and the rotation
//! run together, since a two-finger touch can do both at once, and each frame returns the events for the app in order:
//! magnify first, then rotate. Pure model: the caller delivers the events however its input path does.

use crate::gesture::{Magnify, Rotate, State};

/// Which two-finger gesture an event belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Magnify,
    Rotate,
}

/// One gesture event for the app. Only `Began`, `Changed` and `Ended` are reported; a gesture that never got past its
/// slop reports nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub state: State,
    /// For `Magnify`, the magnification (0.0 is no change). For `Rotate`, the radians turned since the fingers landed.
    pub value: f32,
}

/// Runs the pinch and the rotation recognizers on the same two-finger touch.
#[derive(Default)]
pub struct TwoFinger {
    magnify: Magnify,
    rotate: Rotate,
}

impl TwoFinger {
    pub fn new() -> Self {
        TwoFinger::default()
    }

    /// One frame with both finger positions. Returns the events this frame produces.
    pub fn frame(&mut self, a: (f32, f32), b: (f32, f32)) -> Vec<Event> {
        let mut out = Vec::new();
        let state = self.magnify.touches(a, b);
        if is_reported(state) {
            out.push(Event { kind: Kind::Magnify, state, value: self.magnify.magnification() });
        }
        let state = self.rotate.touches(a, b);
        if is_reported(state) {
            out.push(Event { kind: Kind::Rotate, state, value: self.rotate.rotation() });
        }
        out
    }

    /// The fingers lifted: an `Ended` event for each gesture that had begun, then both recognizers start over.
    pub fn lift(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        let magnification = self.magnify.magnification();
        if self.magnify.up() == State::Ended {
            out.push(Event { kind: Kind::Magnify, state: State::Ended, value: magnification });
        }
        let rotation = self.rotate.rotation();
        if self.rotate.up() == State::Ended {
            out.push(Event { kind: Kind::Rotate, state: State::Ended, value: rotation });
        }
        out
    }
}

fn is_reported(state: State) -> bool {
    matches!(state, State::Began | State::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_pinch_apart_begins_then_changes_and_ends_on_lift() {
        let mut t = TwoFinger::new();
        assert!(t.frame((0.0, 0.0), (100.0, 0.0)).is_empty(), "the first frame only records the spacing");
        let began = t.frame((0.0, 0.0), (200.0, 0.0));
        assert_eq!(began.len(), 1);
        assert_eq!(began[0].kind, Kind::Magnify);
        assert_eq!(began[0].state, State::Began);
        assert!(close(began[0].value, 1.0), "twice the starting spacing");
        let changed = t.frame((0.0, 0.0), (150.0, 0.0));
        assert_eq!(changed, vec![Event { kind: Kind::Magnify, state: State::Changed, value: 0.5 }]);
        let ended = t.lift();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].state, State::Ended);
        assert!(close(ended[0].value, 0.5), "the lift reports the last magnification");
    }

    #[test]
    fn small_movement_inside_the_slop_reports_nothing() {
        let mut t = TwoFinger::new();
        t.frame((0.0, 0.0), (100.0, 0.0));
        assert!(t.frame((0.0, 0.0), (102.0, 0.0)).is_empty());
        assert!(t.lift().is_empty(), "a gesture that never began ends silently");
    }

    #[test]
    fn a_quarter_turn_begins_a_rotation_in_radians() {
        let mut t = TwoFinger::new();
        t.frame((0.0, 0.0), (100.0, 0.0));
        let began = t.frame((0.0, 0.0), (0.0, 100.0));
        let rotate: Vec<&Event> = began.iter().filter(|e| e.kind == Kind::Rotate).collect();
        assert_eq!(rotate.len(), 1);
        assert_eq!(rotate[0].state, State::Began);
        assert!(close(rotate[0].value, std::f32::consts::FRAC_PI_2));
    }

    #[test]
    fn pinch_and_rotation_in_one_frame_come_out_magnify_first() {
        let mut t = TwoFinger::new();
        t.frame((0.0, 0.0), (100.0, 0.0));
        let events = t.frame((0.0, 0.0), (0.0, 200.0));
        let kinds: Vec<Kind> = events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Kind::Magnify, Kind::Rotate]);
        assert!(events.iter().all(|e| e.state == State::Began));
    }

    #[test]
    fn lift_ends_only_the_gestures_that_began_and_resets_them() {
        let mut t = TwoFinger::new();
        t.frame((0.0, 0.0), (100.0, 0.0));
        t.frame((0.0, 0.0), (0.0, 200.0)); // pinch and rotation both began
        let ended = t.lift();
        assert_eq!(ended.len(), 2);
        assert!(ended.iter().all(|e| e.state == State::Ended));
        assert!(t.lift().is_empty(), "nothing is active after the lift");
        // A new touch starts from its own spacing, not the old one.
        assert!(t.frame((0.0, 0.0), (50.0, 0.0)).is_empty());
        let again = t.frame((0.0, 0.0), (100.0, 0.0));
        assert!(close(again[0].value, 1.0));
    }

    #[test]
    fn fingers_on_one_point_have_no_spacing_to_scale_from() {
        let mut t = TwoFinger::new();
        assert!(t.frame((5.0, 5.0), (5.0, 5.0)).is_empty());
        assert!(t.frame((5.0, 5.0), (60.0, 5.0)).is_empty(), "the pinch failed for good in this touch");
        assert!(t.lift().is_empty());
    }
}
