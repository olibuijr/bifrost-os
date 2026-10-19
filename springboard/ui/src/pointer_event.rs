//! Pointer events an app gets besides touches: the scroll wheel (`scrollWheel`), the pointer moving over the window
//! (`mouseMoved`, `mouseExited`), and the two-finger magnify and rotate gestures. The shell sends them as
//! `ToApp::Pointer`; an app that wants them says so with an interest mask (`ToShell::PointerInterest`), so an app that
//! does not ask keeps the touch stream it always had (the wheel as a drag, no hover).
//!
//! [`PointerRouter`] is what an app keeps: it turns the hover events into the enter, exit and move events of its
//! tracking areas ([`crate::tracking`]), and passes the wheel and the gestures on with their location. Positions
//! are in surface pixels, as for touches; [`PointerEvent::to_points`] converts to points.

use crate::tracking::{self, Area, Tracker};
use crate::wheel::{Phase, ScrollEvent};

/// Interest bit: the scroll wheel.
pub const WHEEL: u32 = 1 << 0;
/// Interest bit: the pointer moving over the window and leaving it.
pub const HOVER: u32 = 1 << 1;
/// Interest bit: the two-finger magnify and rotate gestures.
pub const GESTURES: u32 = 1 << 2;
/// Every bit this version knows.
pub const ALL: u32 = WHEEL | HOVER | GESTURES;

/// One pointer event, at (`x`, `y`) in the window's surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerEvent {
    /// The wheel or two-finger scroll. `dx` and `dy` are how far the content moves in pixels (the direction the fingers
    /// go): a positive `dy` moves the content down, so a view scrolls toward its start. `precise` is true for a
    /// touchpad and false for a notched wheel (`hasPreciseScrollingDeltas`).
    Wheel { x: f32, y: f32, dx: f32, dy: f32, precise: bool, phase: Phase, momentum: Phase },
    /// The pointer moved over the window with no button down.
    Hover { x: f32, y: f32 },
    /// The pointer left the window (or went over another window).
    HoverExit,
    /// `magnify(with:)`: `magnification` is 0.0 for no change, 1.0 for double the size.
    Magnify { x: f32, y: f32, magnification: f32, phase: Phase },
    /// `rotate(with:)`: `radians` turned since the fingers landed.
    Rotate { x: f32, y: f32, radians: f32, phase: Phase },
}

impl PointerEvent {
    /// The interest bit an app needs for this event.
    pub fn interest(&self) -> u32 {
        match self {
            PointerEvent::Wheel { .. } => WHEEL,
            PointerEvent::Hover { .. } | PointerEvent::HoverExit => HOVER,
            PointerEvent::Magnify { .. } | PointerEvent::Rotate { .. } => GESTURES,
        }
    }

    /// The same event with its location (and a wheel's deltas) divided by `scale`: pixels to points. A scale that is
    /// not positive and finite leaves the event as it is.
    pub fn to_points(self, scale: f32) -> PointerEvent {
        if !(scale.is_finite() && scale > 0.0) {
            return self;
        }
        match self {
            PointerEvent::Wheel { x, y, dx, dy, precise, phase, momentum } => PointerEvent::Wheel {
                x: x / scale,
                y: y / scale,
                dx: dx / scale,
                dy: dy / scale,
                precise,
                phase,
                momentum,
            },
            PointerEvent::Hover { x, y } => PointerEvent::Hover { x: x / scale, y: y / scale },
            PointerEvent::HoverExit => PointerEvent::HoverExit,
            PointerEvent::Magnify { x, y, magnification, phase } => {
                PointerEvent::Magnify { x: x / scale, y: y / scale, magnification, phase }
            }
            PointerEvent::Rotate { x, y, radians, phase } => PointerEvent::Rotate { x: x / scale, y: y / scale, radians, phase },
        }
    }
}

/// The wire byte of a phase.
pub fn phase_to_wire(p: Phase) -> u8 {
    match p {
        Phase::None => 0,
        Phase::Began => 1,
        Phase::Changed => 2,
        Phase::Ended => 3,
        Phase::Cancelled => 4,
        Phase::MayBegin => 5,
    }
}

/// The phase a wire byte stands for, or `None` for a byte no phase has.
pub fn phase_from_wire(b: u8) -> Option<Phase> {
    Some(match b {
        0 => Phase::None,
        1 => Phase::Began,
        2 => Phase::Changed,
        3 => Phase::Ended,
        4 => Phase::Cancelled,
        5 => Phase::MayBegin,
        _ => return None,
    })
}

/// What a [`PointerRouter`] hands the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delivered {
    /// An enter, exit or move of a tracking area.
    Tracking(tracking::Event),
    /// `scrollWheel(with:)` at (`x`, `y`).
    Scroll { x: f32, y: f32, event: ScrollEvent },
    /// `magnify(with:)`.
    Magnify { x: f32, y: f32, magnification: f32, phase: Phase },
    /// `rotate(with:)`.
    Rotate { x: f32, y: f32, radians: f32, phase: Phase },
}

/// The app's side of pointer input: the tracking areas and the routing of each event.
#[derive(Clone, Debug)]
pub struct PointerRouter {
    tracker: Tracker,
}

impl Default for PointerRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl PointerRouter {
    pub fn new() -> Self {
        PointerRouter { tracker: Tracker::new() }
    }

    /// Replaces the tracking areas (`updateTrackingAreas`). An area that goes while the pointer is in it exits now.
    pub fn set_areas(&mut self, areas: Vec<Area>) -> Vec<Delivered> {
        self.tracker.update(areas).into_iter().map(Delivered::Tracking).collect()
    }

    /// The window became or stopped being key (for areas that are active only in the key window).
    pub fn set_key_window(&mut self, key: bool) -> Vec<Delivered> {
        self.tracker.set_key_window(key).into_iter().map(Delivered::Tracking).collect()
    }

    /// Routes one event. Positions are in the same space as the area rectangles, so convert with
    /// [`PointerEvent::to_points`] first when the areas are in points.
    pub fn route(&mut self, ev: &PointerEvent) -> Vec<Delivered> {
        match *ev {
            PointerEvent::Hover { x, y } => self.tracker.pointer(x, y).into_iter().map(Delivered::Tracking).collect(),
            PointerEvent::HoverExit => self.tracker.pointer_left().into_iter().map(Delivered::Tracking).collect(),
            PointerEvent::Wheel { x, y, dx, dy, precise, phase, momentum } => vec![Delivered::Scroll {
                x,
                y,
                event: ScrollEvent {
                    scrolling_delta_x: dx,
                    scrolling_delta_y: dy,
                    has_precise_scrolling_deltas: precise,
                    phase,
                    momentum_phase: momentum,
                },
            }],
            PointerEvent::Magnify { x, y, magnification, phase } => vec![Delivered::Magnify { x, y, magnification, phase }],
            PointerEvent::Rotate { x, y, radians, phase } => vec![Delivered::Rotate { x, y, radians, phase }],
        }
    }
}

/// The pointer events of a two-finger gesture frame (`trackpad_gesture`), placed at the midpoint of the fingers.
pub fn from_gesture(ev: &crate::trackpad_gesture::Event, at: (f32, f32)) -> PointerEvent {
    use crate::gesture::State;
    use crate::trackpad_gesture::Kind;
    let phase = match ev.state {
        State::Began => Phase::Began,
        State::Changed => Phase::Changed,
        State::Ended => Phase::Ended,
        State::Possible | State::Failed => Phase::Cancelled,
    };
    match ev.kind {
        Kind::Magnify => PointerEvent::Magnify { x: at.0, y: at.1, magnification: ev.value, phase },
        Kind::Rotate => PointerEvent::Rotate { x: at.0, y: at.1, radians: ev.value, phase },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracking::{ENTERED_EXITED, MOVED};
    use crate::Rect;

    fn area(id: u64, x: f32, y: f32, w: f32, h: f32, options: u32) -> Area {
        Area { id, rect: Rect { x, y, w, h }, options }
    }

    #[test]
    fn every_phase_round_trips_through_its_wire_byte() {
        for p in [Phase::None, Phase::Began, Phase::Changed, Phase::Ended, Phase::Cancelled, Phase::MayBegin] {
            assert_eq!(phase_from_wire(phase_to_wire(p)), Some(p));
        }
        assert_eq!(phase_from_wire(6), None);
        assert_eq!(phase_from_wire(255), None);
    }

    #[test]
    fn each_event_names_the_interest_it_needs() {
        let wheel = PointerEvent::Wheel { x: 0.0, y: 0.0, dx: 0.0, dy: 1.0, precise: false, phase: Phase::None, momentum: Phase::None };
        assert_eq!(wheel.interest(), WHEEL);
        assert_eq!(PointerEvent::Hover { x: 1.0, y: 1.0 }.interest(), HOVER);
        assert_eq!(PointerEvent::HoverExit.interest(), HOVER);
        assert_eq!(PointerEvent::Magnify { x: 0.0, y: 0.0, magnification: 0.1, phase: Phase::Began }.interest(), GESTURES);
        assert_eq!(PointerEvent::Rotate { x: 0.0, y: 0.0, radians: 0.1, phase: Phase::Began }.interest(), GESTURES);
        assert_eq!(WHEEL | HOVER | GESTURES, ALL);
    }

    #[test]
    fn pixels_become_points_for_the_location_and_the_wheel_deltas() {
        let precise = PointerEvent::Wheel { x: 30.0, y: 60.0, dx: 4.0, dy: 8.0, precise: true, phase: Phase::Changed, momentum: Phase::None };
        let notched = PointerEvent::Wheel { x: 30.0, y: 60.0, dx: 0.0, dy: 30.0, precise: false, phase: Phase::None, momentum: Phase::None };
        assert_eq!(
            precise.to_points(2.0),
            PointerEvent::Wheel { x: 15.0, y: 30.0, dx: 2.0, dy: 4.0, precise: true, phase: Phase::Changed, momentum: Phase::None }
        );
        assert_eq!(
            notched.to_points(2.0),
            PointerEvent::Wheel { x: 15.0, y: 30.0, dx: 0.0, dy: 15.0, precise: false, phase: Phase::None, momentum: Phase::None }
        );
        assert_eq!(PointerEvent::Hover { x: 10.0, y: 20.0 }.to_points(2.0), PointerEvent::Hover { x: 5.0, y: 10.0 });
        assert_eq!(PointerEvent::Hover { x: 10.0, y: 20.0 }.to_points(0.0), PointerEvent::Hover { x: 10.0, y: 20.0 });
        assert_eq!(PointerEvent::Hover { x: 10.0, y: 20.0 }.to_points(f32::NAN), PointerEvent::Hover { x: 10.0, y: 20.0 });
        assert_eq!(PointerEvent::HoverExit.to_points(2.0), PointerEvent::HoverExit);
    }

    #[test]
    fn hover_drives_the_tracking_areas_and_leaving_exits_them() {
        let mut r = PointerRouter::new();
        assert!(r.set_areas(vec![area(1, 0.0, 0.0, 100.0, 100.0, ENTERED_EXITED | MOVED)]).is_empty());
        let t = |e| Delivered::Tracking(e);
        assert_eq!(
            r.route(&PointerEvent::Hover { x: 5.0, y: 5.0 }),
            vec![t(tracking::Event::Entered(1)), t(tracking::Event::Moved(1, 5.0, 5.0))]
        );
        assert_eq!(r.route(&PointerEvent::Hover { x: 6.0, y: 5.0 }), vec![t(tracking::Event::Moved(1, 6.0, 5.0))]);
        assert_eq!(r.route(&PointerEvent::HoverExit), vec![t(tracking::Event::Exited(1))]);
        assert!(r.route(&PointerEvent::HoverExit).is_empty(), "nothing left to exit");
        // a tracked area that is removed while hovered exits at once
        r.route(&PointerEvent::Hover { x: 5.0, y: 5.0 });
        assert_eq!(r.set_areas(vec![]), vec![t(tracking::Event::Exited(1))]);
    }

    #[test]
    fn the_wheel_reaches_the_app_with_its_location_and_phases() {
        let mut r = PointerRouter::new();
        let out = r.route(&PointerEvent::Wheel { x: 12.0, y: 34.0, dx: 0.0, dy: -7.0, precise: true, phase: Phase::Began, momentum: Phase::None });
        assert_eq!(out.len(), 1);
        let Delivered::Scroll { x, y, event } = out[0] else { panic!("a scroll event") };
        assert_eq!((x, y), (12.0, 34.0));
        assert_eq!((event.scrolling_delta_x, event.scrolling_delta_y), (0.0, -7.0));
        assert!(event.has_precise_scrolling_deltas);
        assert_eq!((event.phase, event.momentum_phase), (Phase::Began, Phase::None));
    }

    #[test]
    fn gestures_pass_through_and_a_gesture_frame_converts_at_the_finger_midpoint() {
        let mut r = PointerRouter::new();
        let ev = PointerEvent::Magnify { x: 1.0, y: 2.0, magnification: 0.25, phase: Phase::Changed };
        assert_eq!(r.route(&ev), vec![Delivered::Magnify { x: 1.0, y: 2.0, magnification: 0.25, phase: Phase::Changed }]);
        let ev = PointerEvent::Rotate { x: 1.0, y: 2.0, radians: 0.5, phase: Phase::Ended };
        assert_eq!(r.route(&ev), vec![Delivered::Rotate { x: 1.0, y: 2.0, radians: 0.5, phase: Phase::Ended }]);

        // two fingers moving apart make a magnify event at their midpoint
        let mut gesture = crate::trackpad_gesture::TwoFinger::new();
        gesture.frame((90.0, 100.0), (110.0, 100.0));
        let mut seen = Vec::new();
        for spread in [30.0_f32, 50.0, 80.0] {
            for e in gesture.frame((100.0 - spread, 100.0), (100.0 + spread, 100.0)) {
                seen.push(from_gesture(&e, (100.0, 100.0)));
            }
        }
        let magnify: Vec<&PointerEvent> = seen.iter().filter(|e| matches!(e, PointerEvent::Magnify { .. })).collect();
        assert!(!magnify.is_empty(), "a pinch past the slop reports magnify events");
        let PointerEvent::Magnify { x, y, magnification, phase } = *magnify[0] else { unreachable!() };
        assert_eq!((x, y), (100.0, 100.0));
        assert!(magnification > 0.0, "fingers moving apart magnify");
        assert_eq!(phase, Phase::Began);
    }

    #[test]
    fn key_window_gating_reaches_the_areas() {
        let mut r = PointerRouter::new();
        r.set_areas(vec![area(1, 0.0, 0.0, 10.0, 10.0, ENTERED_EXITED | tracking::ACTIVE_IN_KEY_WINDOW)]);
        r.route(&PointerEvent::Hover { x: 1.0, y: 1.0 });
        assert_eq!(r.set_key_window(false), vec![Delivered::Tracking(tracking::Event::Exited(1))]);
        assert!(r.route(&PointerEvent::Hover { x: 2.0, y: 2.0 }).is_empty());
        assert!(r.set_key_window(true).is_empty());
    }
}
