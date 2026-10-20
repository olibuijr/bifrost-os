//! Touch input of the shell's Wayland client path: `wl_touch` events turned into the shell's input events.
//!
//! The compositor reports each finger with an id that is unique while the finger is down, in coordinates local to
//! the surface it touched, and groups the events of one hardware report with `frame`. [`Translator`] follows the
//! fingers and produces, in the same units the pointer path uses (screen pixels, the surface's shadow margin
//! removed and its position added):
//!
//! - an [`Event::Contact`] for every finger, with its id and phase (`Down`, `Move`, `Up`, `Cancel`);
//! - the single-pointer [`Event::Down`] / [`Event::Move`] / [`Event::Up`] for the primary finger, the first one of a
//!   touch sequence, exactly as the evdev decoder (`input::Decoder`) does for slot 0. A second finger never takes
//!   over as primary, even after the first one lifts, until every finger is up.
//!
//! Motion is coalesced per frame: a finger that moved several times in one report yields one `Move`. The module is
//! pure (no socket, no clock; the caller passes the time), so it is host-tested. `wayland.rs` owns the `Dispatch`
//! glue that feeds it.
use super::input::{Event, TouchPhase};

/// Where a surface sits on screen, to turn surface-local touch coordinates into screen pixels the way pointer
/// coordinates are: `screen = local - margin + offset`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// The surface's top-left corner on screen.
    pub offset: (f32, f32),
    /// The transparent shadow margin around a window's content in its drawable.
    pub margin: f32,
}

impl Placement {
    /// Surface-local coordinates with the shadow margin removed (the window's own content coordinates).
    pub fn local(self, x: f64, y: f64) -> (f32, f32) {
        (x as f32 - self.margin, y as f32 - self.margin)
    }

    /// Surface-local coordinates in screen pixels.
    pub fn to_screen(self, x: f64, y: f64) -> (f32, f32) {
        let (lx, ly) = self.local(x, y);
        (lx + self.offset.0, ly + self.offset.1)
    }
}

/// Whether a touch at `local` (window content px) starts a window move: it lands in the title area above
/// `drag_height` and outside the rectangles `no_drag` (x, y, w, h), which hold the traffic lights (the shell derives
/// them from the lights' own rectangles and the scale) and an app's toolbar controls.
pub fn starts_window_drag(local: (f32, f32), drag_height: i32, no_drag: &[[i32; 4]]) -> bool {
    let (x, y) = (local.0 as i32, local.1 as i32);
    local.1 < drag_height as f32 && !no_drag.iter().any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
}

struct Contact {
    id: i32,
    place: Placement,
    /// Last position, screen px.
    pos: (f32, f32),
    /// Moved since the last frame, and when.
    moved: Option<f64>,
}

/// Follows the fingers on a `wl_touch` and produces shell input events.
#[derive(Default)]
pub struct Translator {
    /// Fingers down, in the order they landed.
    active: Vec<Contact>,
    /// The id of the finger that drives the single-pointer events.
    primary: Option<i32>,
    ready: Vec<Event>,
}

impl Translator {
    /// True when no finger is down: a finger landing now would be the primary one.
    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }

    /// The ids of the fingers down, in landing order.
    pub fn active_ids(&self) -> Vec<i32> {
        self.active.iter().map(|c| c.id).collect()
    }

    /// A finger landed at surface-local (`x`, `y`) on a surface at `place`. A repeated id is ignored.
    pub fn down(&mut self, id: i32, x: f64, y: f64, place: Placement, t: f64) {
        if self.active.iter().any(|c| c.id == id) {
            return;
        }
        let pos = place.to_screen(x, y);
        if self.active.is_empty() {
            self.primary = Some(id);
        }
        self.active.push(Contact { id, place, pos, moved: None });
        self.ready.push(Event::Contact { id, phase: TouchPhase::Down, x: pos.0, y: pos.1, t });
        if self.primary == Some(id) {
            self.ready.push(Event::Down { x: pos.0, y: pos.1, t });
        }
    }

    /// A finger moved to surface-local (`x`, `y`). The move is reported at the next [`Translator::frame`]; an unknown
    /// id and a move that stays on the same pixel are ignored.
    pub fn motion(&mut self, id: i32, x: f64, y: f64, t: f64) {
        let Some(c) = self.active.iter_mut().find(|c| c.id == id) else { return };
        let pos = c.place.to_screen(x, y);
        if pos != c.pos {
            c.pos = pos;
            c.moved = Some(t);
        }
    }

    /// A finger lifted. It ends at its last position; a move not yet reported is folded into the end.
    pub fn up(&mut self, id: i32, t: f64) {
        let Some(i) = self.active.iter().position(|c| c.id == id) else { return };
        let c = self.active.remove(i);
        self.ready.push(Event::Contact { id, phase: TouchPhase::Up, x: c.pos.0, y: c.pos.1, t });
        if self.primary == Some(id) {
            self.ready.push(Event::Up { x: c.pos.0, y: c.pos.1, t });
            self.primary = None;
        }
    }

    /// The end of one report: every finger that moved since the last one gets a single `Move`.
    pub fn frame(&mut self) {
        for c in &mut self.active {
            let Some(t) = c.moved.take() else { continue };
            self.ready.push(Event::Contact { id: c.id, phase: TouchPhase::Move, x: c.pos.0, y: c.pos.1, t });
            if self.primary == Some(c.id) {
                self.ready.push(Event::Move { x: c.pos.0, y: c.pos.1, t });
            }
        }
    }

    /// The compositor took every finger over (or the seat lost touch). Each finger ends with a `Cancel`. The
    /// shell has no cancel of its own, so the primary finger also ends with an `Up` where it was, like a pointer
    /// leaving the surface mid-press.
    pub fn cancel(&mut self, t: f64) {
        for c in self.active.drain(..) {
            self.ready.push(Event::Contact { id: c.id, phase: TouchPhase::Cancel, x: c.pos.0, y: c.pos.1, t });
            if self.primary == Some(c.id) {
                self.ready.push(Event::Up { x: c.pos.0, y: c.pos.1, t });
            }
        }
        self.primary = None;
    }

    /// Hands over the events produced so far, in order.
    pub fn take(&mut self, out: &mut Vec<Event>) {
        out.append(&mut self.ready);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT_ORIGIN: Placement = Placement { offset: (0.0, 0.0), margin: 0.0 };

    fn drain(t: &mut Translator) -> Vec<Event> {
        let mut out = Vec::new();
        t.take(&mut out);
        out
    }

    fn contact(id: i32, phase: TouchPhase, x: f32, y: f32, t: f64) -> Event {
        Event::Contact { id, phase, x, y, t }
    }

    #[test]
    fn placement_removes_margin_and_adds_offset() {
        let p = Placement { offset: (100.0, 50.0), margin: 40.0 };
        assert_eq!(p.local(60.0, 70.0), (20.0, 30.0));
        assert_eq!(p.to_screen(60.0, 70.0), (120.0, 80.0));
        assert_eq!(AT_ORIGIN.to_screen(3.5, 4.25), (3.5, 4.25));
    }

    #[test]
    fn single_finger_tap_matches_the_pointer_events() {
        let mut t = Translator::default();
        t.down(7, 10.0, 20.0, AT_ORIGIN, 1.0);
        t.frame();
        t.up(7, 5.0);
        t.frame();
        assert_eq!(
            drain(&mut t),
            vec![
                contact(7, TouchPhase::Down, 10.0, 20.0, 1.0),
                Event::Down { x: 10.0, y: 20.0, t: 1.0 },
                contact(7, TouchPhase::Up, 10.0, 20.0, 5.0),
                Event::Up { x: 10.0, y: 20.0, t: 5.0 },
            ]
        );
        assert!(t.is_idle());
    }

    #[test]
    fn drag_reports_one_move_per_frame_at_the_latest_position() {
        let mut t = Translator::default();
        t.down(0, 0.0, 0.0, AT_ORIGIN, 0.0);
        drain(&mut t);
        t.motion(0, 5.0, 5.0, 8.0);
        t.motion(0, 9.0, 12.0, 12.0);
        assert!(drain(&mut t).is_empty(), "nothing before the frame");
        t.frame();
        assert_eq!(drain(&mut t), vec![contact(0, TouchPhase::Move, 9.0, 12.0, 12.0), Event::Move { x: 9.0, y: 12.0, t: 12.0 }]);
        t.frame();
        assert!(drain(&mut t).is_empty(), "a quiet frame reports nothing");
    }

    #[test]
    fn a_move_to_the_same_pixel_is_dropped() {
        let mut t = Translator::default();
        t.down(0, 4.0, 4.0, AT_ORIGIN, 0.0);
        drain(&mut t);
        t.motion(0, 4.0, 4.0, 3.0);
        t.frame();
        assert!(drain(&mut t).is_empty());
    }

    #[test]
    fn up_folds_in_an_unreported_move() {
        let mut t = Translator::default();
        t.down(0, 0.0, 0.0, AT_ORIGIN, 0.0);
        drain(&mut t);
        t.motion(0, 30.0, 40.0, 4.0);
        t.up(0, 6.0);
        t.frame();
        assert_eq!(drain(&mut t), vec![contact(0, TouchPhase::Up, 30.0, 40.0, 6.0), Event::Up { x: 30.0, y: 40.0, t: 6.0 }]);
    }

    #[test]
    fn second_finger_is_a_contact_only() {
        let mut t = Translator::default();
        t.down(1, 10.0, 10.0, AT_ORIGIN, 0.0);
        t.down(2, 200.0, 300.0, AT_ORIGIN, 1.0);
        assert_eq!(
            drain(&mut t),
            vec![
                contact(1, TouchPhase::Down, 10.0, 10.0, 0.0),
                Event::Down { x: 10.0, y: 10.0, t: 0.0 },
                contact(2, TouchPhase::Down, 200.0, 300.0, 1.0),
            ]
        );
        assert_eq!(t.active_ids(), vec![1, 2]);
        t.motion(2, 210.0, 310.0, 2.0);
        t.motion(1, 12.0, 14.0, 2.0);
        t.frame();
        assert_eq!(
            drain(&mut t),
            vec![
                contact(1, TouchPhase::Move, 12.0, 14.0, 2.0),
                Event::Move { x: 12.0, y: 14.0, t: 2.0 },
                contact(2, TouchPhase::Move, 210.0, 310.0, 2.0),
            ]
        );
    }

    #[test]
    fn primary_ends_with_its_finger_and_is_not_handed_on() {
        let mut t = Translator::default();
        t.down(1, 0.0, 0.0, AT_ORIGIN, 0.0);
        t.down(2, 50.0, 50.0, AT_ORIGIN, 1.0);
        drain(&mut t);
        t.up(1, 2.0);
        t.motion(2, 60.0, 60.0, 3.0);
        t.frame();
        assert_eq!(
            drain(&mut t),
            vec![contact(1, TouchPhase::Up, 0.0, 0.0, 2.0), Event::Up { x: 0.0, y: 0.0, t: 2.0 }, contact(2, TouchPhase::Move, 60.0, 60.0, 3.0)]
        );
        // a new finger while the second is still down is not primary either
        t.down(3, 5.0, 5.0, AT_ORIGIN, 4.0);
        assert_eq!(drain(&mut t), vec![contact(3, TouchPhase::Down, 5.0, 5.0, 4.0)]);
        t.up(2, 5.0);
        t.up(3, 6.0);
        drain(&mut t);
        assert!(t.is_idle());
        // once every finger is up the next one is primary again
        t.down(4, 1.0, 2.0, AT_ORIGIN, 7.0);
        assert!(drain(&mut t).contains(&Event::Down { x: 1.0, y: 2.0, t: 7.0 }));
    }

    #[test]
    fn contacts_keep_the_surface_they_landed_on() {
        let window = Placement { offset: (100.0, 200.0), margin: 40.0 };
        let mut t = Translator::default();
        t.down(0, 50.0, 60.0, window, 0.0);
        t.down(1, 30.0, 30.0, AT_ORIGIN, 0.0);
        t.motion(0, 90.0, 100.0, 1.0);
        t.motion(1, 31.0, 32.0, 1.0);
        drain(&mut t);
        t.frame();
        assert_eq!(
            drain(&mut t),
            vec![contact(0, TouchPhase::Move, 150.0, 260.0, 1.0), Event::Move { x: 150.0, y: 260.0, t: 1.0 }, contact(1, TouchPhase::Move, 31.0, 32.0, 1.0)]
        );
    }

    #[test]
    fn unknown_ids_and_repeated_downs_are_ignored() {
        let mut t = Translator::default();
        t.motion(9, 1.0, 1.0, 0.0);
        t.up(9, 0.0);
        t.frame();
        assert!(drain(&mut t).is_empty());
        t.down(3, 1.0, 1.0, AT_ORIGIN, 0.0);
        let first = drain(&mut t);
        t.down(3, 9.0, 9.0, AT_ORIGIN, 1.0);
        assert_eq!(first.len(), 2);
        assert!(drain(&mut t).is_empty());
        assert_eq!(t.active_ids(), vec![3]);
    }

    #[test]
    fn cancel_ends_every_finger_and_the_primary_pointer_press() {
        let mut t = Translator::default();
        t.down(1, 10.0, 10.0, AT_ORIGIN, 0.0);
        t.down(2, 20.0, 20.0, AT_ORIGIN, 0.0);
        t.motion(2, 25.0, 25.0, 1.0);
        drain(&mut t);
        t.cancel(9.0);
        assert_eq!(
            drain(&mut t),
            vec![contact(1, TouchPhase::Cancel, 10.0, 10.0, 9.0), Event::Up { x: 10.0, y: 10.0, t: 9.0 }, contact(2, TouchPhase::Cancel, 25.0, 25.0, 9.0)]
        );
        assert!(t.is_idle());
        t.frame();
        assert!(drain(&mut t).is_empty(), "a move pending at the cancel is dropped");
        t.cancel(10.0);
        assert!(drain(&mut t).is_empty(), "cancelling with no finger down reports nothing");
    }

    #[test]
    fn title_area_press_starts_a_window_drag() {
        let no_drag = [[0, 0, 80, 30], [100, 0, 40, 30]];
        assert!(starts_window_drag((300.0, 10.0), 30, &no_drag));
        assert!(!starts_window_drag((300.0, 40.0), 30, &no_drag), "below the title bar");
        assert!(!starts_window_drag((40.0, 10.0), 30, &no_drag), "on the traffic lights");
        assert!(!starts_window_drag((110.0, 10.0), 30, &no_drag), "on a toolbar control");
        assert!(starts_window_drag((140.0, 10.0), 30, &no_drag), "just past the control");
    }
}
