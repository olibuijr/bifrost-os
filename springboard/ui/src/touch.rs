//! Touch objects and events: what an app sees of fingers, a stylus or an indirect pointer on its surface.
//!
//! A [`Touch`] is one contact followed from its first sample to its last: a stable `id`, a [`TouchPhase`], the location
//! and the previous location, a timestamp, a tap count, and optionally the force, the maximum possible force and the
//! major radius of the contact. A [`TouchEvent`] groups every touch of one frame, so a two-finger pinch arrives as one
//! event holding both fingers, the one that moved and the one that did not.
//!
//! [`TouchTracker`] builds those events from raw samples (down, move, up, cancel). It is a pure state machine driven by
//! the caller's clock, shared by the shell (which owns the real input) and by the SDK (which can derive a single
//! indirect touch from a plain pointer). Coordinates are in whatever unit the caller uses (surface pixels on the
//! wire); use [`TouchEvent::to_points`] to convert. Times are milliseconds on one monotonic clock.

/// Where a touch is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    /// the contact started in this frame
    Began = 0,
    /// the contact moved
    Moved = 1,
    /// the contact is down and did not move in this frame
    Stationary = 2,
    /// the contact lifted
    Ended = 3,
    /// the system took the contact away (another view claimed it, or the window lost the pointer)
    Cancelled = 4,
}

impl TouchPhase {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Began,
            1 => Self::Moved,
            2 => Self::Stationary,
            3 => Self::Ended,
            4 => Self::Cancelled,
            _ => return None,
        })
    }

    /// True for the phases after which the touch no longer exists.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Ended | Self::Cancelled)
    }
}

/// What made the contact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TouchType {
    /// a finger on the screen
    #[default]
    Direct = 0,
    /// a stylus
    Pencil = 1,
    /// a mouse, trackpad or other pointer that is not on the surface itself
    Indirect = 2,
}

impl TouchType {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Direct,
            1 => Self::Pencil,
            2 => Self::Indirect,
            _ => return None,
        })
    }
}

/// One contact, as of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    /// stable from `Began` to `Ended` / `Cancelled`; reused only after the touch is gone
    pub id: u32,
    pub phase: TouchPhase,
    pub kind: TouchType,
    /// location in the surface's coordinates
    pub x: f32,
    pub y: f32,
    /// the location in the previous frame that carried this touch (equal to `x`, `y` for `Began` and `Stationary`)
    pub prev_x: f32,
    pub prev_y: f32,
    /// ms, when this touch last changed
    pub timestamp: f64,
    /// 1 for a single tap, 2 for the second tap in quick succession at the same place, and so on
    pub tap_count: u32,
    /// the pressure of the contact; `None` when the hardware has no pressure
    pub force: Option<f32>,
    /// the largest `force` the hardware reports; `None` together with `force`
    pub max_force: Option<f32>,
    /// the major radius of the contact ellipse; `None` when the hardware has no contact size
    pub major_radius: Option<f32>,
}

impl Touch {
    pub fn location(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    pub fn previous_location(&self) -> (f32, f32) {
        (self.prev_x, self.prev_y)
    }

    /// How far the touch moved since the previous frame.
    pub fn delta(&self) -> (f32, f32) {
        (self.x - self.prev_x, self.y - self.prev_y)
    }

    /// `force / max_force`, clamped to 0..=1; `None` without pressure data or with a zero maximum.
    pub fn normalized_force(&self) -> Option<f32> {
        match (self.force, self.max_force) {
            (Some(f), Some(m)) if m > 0.0 => Some((f / m).clamp(0.0, 1.0)),
            _ => None,
        }
    }

    /// The same touch with the location, previous location and radius multiplied by `k` (px to points: `1 / scale`).
    pub fn scaled(&self, k: f32) -> Touch {
        Touch { x: self.x * k, y: self.y * k, prev_x: self.prev_x * k, prev_y: self.prev_y * k, major_radius: self.major_radius.map(|r| r * k), ..*self }
    }
}

/// The most touches one event carries; a frame with more drops the extra (the ones with the highest ids).
pub const MAX_TOUCHES: usize = 16;

/// Every touch of one frame: the ones that changed and the ones that are down and did not.
#[derive(Clone, Debug, PartialEq)]
pub struct TouchEvent {
    /// ms, when the frame was sampled
    pub timestamp: f64,
    pub touches: Vec<Touch>,
}

impl TouchEvent {
    /// The touches in `phase`.
    pub fn in_phase(&self, phase: TouchPhase) -> impl Iterator<Item = &Touch> {
        self.touches.iter().filter(move |t| t.phase == phase)
    }

    pub fn touch(&self, id: u32) -> Option<&Touch> {
        self.touches.iter().find(|t| t.id == id)
    }

    /// True when no touch of the frame stays down afterwards.
    pub fn all_finished(&self) -> bool {
        self.touches.iter().all(|t| t.phase.is_final())
    }

    /// True when dropping this frame loses nothing the next one does not carry: only moves and stationary touches.
    pub fn droppable(&self) -> bool {
        !self.touches.is_empty() && self.touches.iter().all(|t| matches!(t.phase, TouchPhase::Moved | TouchPhase::Stationary))
    }

    /// The same event with every location multiplied by `k` (px to points: `1 / scale`).
    pub fn to_points(&self, scale: f32) -> TouchEvent {
        let k = if scale > 0.0 { 1.0 / scale } else { 1.0 };
        TouchEvent { timestamp: self.timestamp, touches: self.touches.iter().map(|t| t.scaled(k)).collect() }
    }

    /// The touches in the coordinates of a view: pixels to points, then the origin of the view (in points of the
    /// window) subtracted, so `(0, 0)` is the view's top left corner.
    pub fn in_view(&self, scale: f32, origin_x: f32, origin_y: f32) -> TouchEvent {
        let mut ev = self.to_points(scale);
        for t in &mut ev.touches {
            t.x -= origin_x;
            t.y -= origin_y;
            t.prev_x -= origin_x;
            t.prev_y -= origin_y;
        }
        ev
    }
}

/// What a raw input device reports for one contact.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub kind: TouchType,
    pub force: Option<f32>,
    pub max_force: Option<f32>,
    pub major_radius: Option<f32>,
}

impl Sample {
    pub fn at(id: u32, x: f32, y: f32) -> Self {
        Sample { id, x, y, ..Sample::default() }
    }

    pub fn with_kind(mut self, kind: TouchType) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_force(mut self, force: f32, max_force: f32) -> Self {
        self.force = Some(force);
        self.max_force = Some(max_force);
        self
    }

    pub fn with_radius(mut self, r: f32) -> Self {
        self.major_radius = Some(r);
        self
    }
}

/// What a device did with a contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    Down,
    Move,
    Up,
    Cancel,
}

/// A contact further than this (in the caller's units) from where it started is not a tap.
pub const TAP_SLOP: f32 = 12.0;
/// A second tap within this many ms of the first, close to it, counts on.
pub const MULTI_TAP_MS: f64 = 400.0;
/// A contact held longer than this is not a tap.
pub const TAP_HOLD_MS: f64 = 500.0;

#[derive(Clone, Copy, Debug)]
struct Active {
    touch: Touch,
    start_x: f32,
    start_y: f32,
    start_t: f64,
    /// the contact has been further than the slop from its start
    wandered: bool,
}

#[derive(Clone, Copy, Debug)]
struct LastTap {
    x: f32,
    y: f32,
    t: f64,
    count: u32,
    kind: TouchType,
}

/// Turns raw samples into [`TouchEvent`]s: phases, previous locations, stationary touches and tap counts.
#[derive(Clone, Debug)]
pub struct TouchTracker {
    active: Vec<Active>,
    last_tap: Option<LastTap>,
    /// tap detection distance
    pub slop: f32,
}

impl Default for TouchTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchTracker {
    pub fn new() -> Self {
        TouchTracker { active: Vec::new(), last_tap: None, slop: TAP_SLOP }
    }

    /// The tracker for a surface whose coordinates are `scale` times the points (so the slop stays the same on screen).
    pub fn with_scale(scale: f32) -> Self {
        let mut t = Self::new();
        if scale > 0.0 {
            t.slop = TAP_SLOP * scale;
        }
        t
    }

    /// Touches currently down.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn is_active(&self, id: u32) -> bool {
        self.active.iter().any(|a| a.touch.id == id)
    }

    /// Applies the changes of one frame at `t` and returns the event, or `None` when nothing changed (a move to the
    /// place the touch already is at, an up of an unknown touch). The event lists the changed touches in the order
    /// given, then every other touch that is down as `Stationary`.
    pub fn frame(&mut self, changes: &[(Raw, Sample)], t: f64) -> Option<TouchEvent> {
        let mut out: Vec<Touch> = Vec::new();
        let mut finished: Vec<u32> = Vec::new();
        for (raw, s) in changes {
            if out.len() >= MAX_TOUCHES || out.iter().any(|o| o.id == s.id) {
                continue;
            }
            match raw {
                Raw::Down => {
                    if self.active.len() >= MAX_TOUCHES && !self.is_active(s.id) {
                        continue;
                    }
                    let touch = self.begin(s, t);
                    out.push(touch);
                }
                Raw::Move => {
                    if let Some(touch) = self.advance(s, t) {
                        out.push(touch);
                    }
                }
                Raw::Up | Raw::Cancel => {
                    if let Some(touch) = self.finish(s, t, *raw == Raw::Cancel) {
                        finished.push(touch.id);
                        out.push(touch);
                    }
                }
            }
        }
        if out.is_empty() {
            return None;
        }
        for a in &self.active {
            if out.len() >= MAX_TOUCHES {
                break;
            }
            if !out.iter().any(|o| o.id == a.touch.id) {
                out.push(Touch { phase: TouchPhase::Stationary, prev_x: a.touch.x, prev_y: a.touch.y, ..a.touch });
            }
        }
        self.active.retain(|a| !finished.contains(&a.touch.id));
        Some(TouchEvent { timestamp: t, touches: out })
    }

    /// One touch went down.
    pub fn down(&mut self, s: Sample, t: f64) -> Option<TouchEvent> {
        self.frame(&[(Raw::Down, s)], t)
    }

    /// One touch moved.
    pub fn moved(&mut self, s: Sample, t: f64) -> Option<TouchEvent> {
        self.frame(&[(Raw::Move, s)], t)
    }

    /// One touch lifted at `s`.
    pub fn up(&mut self, s: Sample, t: f64) -> Option<TouchEvent> {
        self.frame(&[(Raw::Up, s)], t)
    }

    /// One touch was taken away.
    pub fn cancel(&mut self, id: u32, t: f64) -> Option<TouchEvent> {
        let at = self.active.iter().find(|a| a.touch.id == id).map(|a| Sample::at(id, a.touch.x, a.touch.y))?;
        self.frame(&[(Raw::Cancel, at)], t)
    }

    /// Every touch that is down was taken away (the window lost the pointer, the app went to the background).
    pub fn cancel_all(&mut self, t: f64) -> Option<TouchEvent> {
        let all: Vec<(Raw, Sample)> = self.active.iter().map(|a| (Raw::Cancel, Sample::at(a.touch.id, a.touch.x, a.touch.y))).collect();
        self.frame(&all, t)
    }

    fn begin(&mut self, s: &Sample, t: f64) -> Touch {
        self.active.retain(|a| a.touch.id != s.id);
        let count = match self.last_tap {
            Some(l) if l.kind == s.kind && t - l.t <= MULTI_TAP_MS && (s.x - l.x).hypot(s.y - l.y) <= self.slop => l.count + 1,
            _ => 1,
        };
        let touch = Touch {
            id: s.id,
            phase: TouchPhase::Began,
            kind: s.kind,
            x: s.x,
            y: s.y,
            prev_x: s.x,
            prev_y: s.y,
            timestamp: t,
            tap_count: count,
            force: s.force,
            max_force: s.max_force,
            major_radius: s.major_radius,
        };
        self.active.push(Active { touch, start_x: s.x, start_y: s.y, start_t: t, wandered: false });
        touch
    }

    fn advance(&mut self, s: &Sample, t: f64) -> Option<Touch> {
        let slop = self.slop;
        let a = self.active.iter_mut().find(|a| a.touch.id == s.id)?;
        let same = a.touch.x == s.x && a.touch.y == s.y && a.touch.force == s.force && a.touch.major_radius == s.major_radius;
        if same {
            return None;
        }
        if (s.x - a.start_x).hypot(s.y - a.start_y) > slop {
            a.wandered = true;
        }
        let moved = Touch {
            phase: TouchPhase::Moved,
            prev_x: a.touch.x,
            prev_y: a.touch.y,
            x: s.x,
            y: s.y,
            timestamp: t,
            force: s.force,
            max_force: s.max_force.or(a.touch.max_force),
            major_radius: s.major_radius,
            ..a.touch
        };
        a.touch = Touch { phase: TouchPhase::Stationary, prev_x: s.x, prev_y: s.y, ..moved };
        Some(moved)
    }

    fn finish(&mut self, s: &Sample, t: f64, cancelled: bool) -> Option<Touch> {
        let slop = self.slop;
        let a = *self.active.iter().find(|a| a.touch.id == s.id)?;
        // the lift position counts for the slop too, unless the touch was taken away
        let (x, y) = if cancelled { (a.touch.x, a.touch.y) } else { (s.x, s.y) };
        let wandered = a.wandered || (x - a.start_x).hypot(y - a.start_y) > slop;
        let phase = if cancelled { TouchPhase::Cancelled } else { TouchPhase::Ended };
        if !cancelled && !wandered && t - a.start_t <= TAP_HOLD_MS {
            self.last_tap = Some(LastTap { x: a.start_x, y: a.start_y, t, count: a.touch.tap_count, kind: a.touch.kind });
        } else {
            self.last_tap = None;
        }
        Some(Touch { phase, prev_x: a.touch.x, prev_y: a.touch.y, x, y, timestamp: t, force: s.force.or(a.touch.force), ..a.touch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: u32, x: f32, y: f32) -> Sample {
        Sample::at(id, x, y)
    }

    #[test]
    fn a_touch_goes_through_began_moved_ended() {
        let mut tr = TouchTracker::new();
        let e = tr.down(s(1, 10.0, 20.0), 100.0).unwrap();
        assert_eq!(e.touches.len(), 1);
        let t = e.touches[0];
        assert_eq!((t.phase, t.location(), t.previous_location(), t.tap_count), (TouchPhase::Began, (10.0, 20.0), (10.0, 20.0), 1));
        let e = tr.moved(s(1, 30.0, 25.0), 116.0).unwrap();
        let t = e.touches[0];
        assert_eq!((t.phase, t.location(), t.previous_location(), t.delta()), (TouchPhase::Moved, (30.0, 25.0), (10.0, 20.0), (20.0, 5.0)));
        assert_eq!(t.timestamp, 116.0);
        let e = tr.moved(s(1, 31.0, 25.0), 132.0).unwrap();
        assert_eq!(e.touches[0].previous_location(), (30.0, 25.0));
        let e = tr.up(s(1, 31.0, 26.0), 140.0).unwrap();
        assert_eq!(e.touches[0].phase, TouchPhase::Ended);
        assert_eq!(e.touches[0].previous_location(), (31.0, 25.0));
        assert!(e.all_finished());
        assert_eq!(tr.active_count(), 0);
    }

    #[test]
    fn a_move_to_the_same_place_is_not_an_event() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 5.0, 5.0), 0.0);
        assert!(tr.moved(s(1, 5.0, 5.0), 10.0).is_none());
        assert!(tr.moved(s(9, 5.0, 5.0), 10.0).is_none(), "unknown touch");
        assert!(tr.up(s(9, 5.0, 5.0), 10.0).is_none(), "unknown touch");
    }

    #[test]
    fn other_fingers_are_stationary_in_the_frame() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 0.0, 0.0), 0.0);
        let e = tr.down(s(2, 100.0, 100.0), 5.0).unwrap();
        assert_eq!(e.touches.len(), 2);
        assert_eq!(e.touches[0].id, 2);
        assert_eq!(e.touches[0].phase, TouchPhase::Began);
        assert_eq!(e.touches[1].id, 1);
        assert_eq!(e.touches[1].phase, TouchPhase::Stationary);
        assert_eq!(e.touches[1].previous_location(), (0.0, 0.0));
        let e = tr.moved(s(2, 110.0, 100.0), 10.0).unwrap();
        assert_eq!(e.in_phase(TouchPhase::Moved).count(), 1);
        assert_eq!(e.touch(1).unwrap().phase, TouchPhase::Stationary);
        assert!(e.droppable());
        let e = tr.up(s(1, 0.0, 0.0), 20.0).unwrap();
        assert_eq!(e.touch(1).unwrap().phase, TouchPhase::Ended);
        assert_eq!(e.touch(2).unwrap().phase, TouchPhase::Stationary);
        assert!(!e.droppable());
        assert!(!e.all_finished());
        assert_eq!(tr.active_count(), 1);
    }

    #[test]
    fn one_frame_can_move_two_fingers() {
        let mut tr = TouchTracker::new();
        tr.frame(&[(Raw::Down, s(1, 0.0, 0.0)), (Raw::Down, s(2, 50.0, 0.0))], 0.0).unwrap();
        let e = tr.frame(&[(Raw::Move, s(1, -5.0, 0.0)), (Raw::Move, s(2, 55.0, 0.0))], 8.0).unwrap();
        assert_eq!(e.touches.len(), 2);
        assert!(e.touches.iter().all(|t| t.phase == TouchPhase::Moved));
        assert_eq!(e.timestamp, 8.0);
    }

    #[test]
    fn a_moved_touch_is_stationary_in_the_next_frame_of_another_finger() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 0.0, 0.0), 0.0);
        tr.moved(s(1, 9.0, 9.0), 4.0);
        let e = tr.down(s(2, 70.0, 70.0), 8.0).unwrap();
        let t = e.touch(1).unwrap();
        assert_eq!((t.phase, t.location(), t.previous_location()), (TouchPhase::Stationary, (9.0, 9.0), (9.0, 9.0)));
        assert_eq!(t.timestamp, 4.0, "the time the touch last changed");
    }

    #[test]
    fn taps_count_up_when_quick_and_close() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 100.0, 100.0), 0.0);
        tr.up(s(1, 100.0, 100.0), 50.0);
        let e = tr.down(s(1, 102.0, 99.0), 200.0).unwrap();
        assert_eq!(e.touches[0].tap_count, 2);
        let e = tr.up(s(1, 102.0, 99.0), 250.0).unwrap();
        assert_eq!(e.touches[0].tap_count, 2, "the lift keeps the count of its press");
        let e = tr.down(s(1, 100.0, 100.0), 400.0).unwrap();
        assert_eq!(e.touches[0].tap_count, 3);
    }

    #[test]
    fn taps_restart_when_slow_far_or_not_taps() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 0.0, 0.0), 0.0);
        tr.up(s(1, 0.0, 0.0), 30.0);
        // too late
        assert_eq!(tr.down(s(1, 0.0, 0.0), 30.0 + MULTI_TAP_MS + 1.0).unwrap().touches[0].tap_count, 1);
        tr.up(s(1, 0.0, 0.0), 500.0);
        // too far
        assert_eq!(tr.down(s(1, 200.0, 0.0), 520.0).unwrap().touches[0].tap_count, 1);
        // a drag is not a tap
        tr.moved(s(1, 260.0, 0.0), 530.0);
        tr.up(s(1, 260.0, 0.0), 540.0);
        assert_eq!(tr.down(s(1, 260.0, 0.0), 560.0).unwrap().touches[0].tap_count, 1);
        // a long press is not a tap
        tr.up(s(1, 260.0, 0.0), 560.0 + TAP_HOLD_MS + 1.0);
        assert_eq!(tr.down(s(1, 260.0, 0.0), 1200.0).unwrap().touches[0].tap_count, 1);
        // a cancelled touch is not a tap
        tr.cancel(1, 1210.0);
        assert_eq!(tr.down(s(1, 260.0, 0.0), 1220.0).unwrap().touches[0].tap_count, 1);
    }

    #[test]
    fn taps_of_different_kinds_do_not_add_up() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 0.0, 0.0), 0.0);
        tr.up(s(1, 0.0, 0.0), 20.0);
        let e = tr.down(s(1, 0.0, 0.0).with_kind(TouchType::Pencil), 60.0).unwrap();
        assert_eq!(e.touches[0].tap_count, 1);
        assert_eq!(e.touches[0].kind, TouchType::Pencil);
    }

    #[test]
    fn the_slop_follows_the_surface_scale() {
        let mut tr = TouchTracker::with_scale(2.0);
        tr.down(s(1, 0.0, 0.0), 0.0);
        tr.up(s(1, 20.0, 0.0), 30.0);
        // 20 px is 10 pt: still within 12 pt, so a tap
        assert_eq!(tr.down(s(1, 20.0, 0.0), 60.0).unwrap().touches[0].tap_count, 2);
    }

    #[test]
    fn cancel_ends_the_touch_without_moving_it() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 1.0, 2.0), 0.0);
        tr.down(s(2, 3.0, 4.0), 1.0);
        let e = tr.cancel(1, 9.0).unwrap();
        let t = e.touch(1).unwrap();
        assert_eq!((t.phase, t.location()), (TouchPhase::Cancelled, (1.0, 2.0)));
        assert_eq!(e.touch(2).unwrap().phase, TouchPhase::Stationary);
        assert!(tr.cancel(1, 10.0).is_none());
        let e = tr.cancel_all(11.0).unwrap();
        assert_eq!(e.touches.len(), 1);
        assert!(e.all_finished());
        assert!(tr.cancel_all(12.0).is_none());
        assert_eq!(tr.active_count(), 0);
    }

    #[test]
    fn force_and_radius_are_carried() {
        let mut tr = TouchTracker::new();
        let e = tr.down(s(1, 0.0, 0.0).with_force(1.0, 4.0).with_radius(6.0), 0.0).unwrap();
        let t = e.touches[0];
        assert_eq!((t.force, t.max_force, t.major_radius), (Some(1.0), Some(4.0), Some(6.0)));
        assert_eq!(t.normalized_force(), Some(0.25));
        // a pressure change alone is a move
        let e = tr.moved(s(1, 0.0, 0.0).with_force(2.0, 4.0).with_radius(6.0), 8.0).unwrap();
        assert_eq!(e.touches[0].phase, TouchPhase::Moved);
        assert_eq!(e.touches[0].force, Some(2.0));
        let plain = tr.down(s(2, 9.0, 9.0), 9.0).unwrap();
        assert_eq!(plain.touch(2).unwrap().normalized_force(), None);
    }

    #[test]
    fn a_second_down_with_the_same_id_starts_a_new_touch() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 0.0, 0.0), 0.0);
        let e = tr.down(s(1, 50.0, 50.0), 10.0).unwrap();
        assert_eq!(e.touches.len(), 1);
        assert_eq!(e.touches[0].phase, TouchPhase::Began);
        assert_eq!(tr.active_count(), 1);
    }

    #[test]
    fn the_number_of_touches_is_capped() {
        let mut tr = TouchTracker::new();
        for i in 0..(MAX_TOUCHES as u32 + 4) {
            tr.down(s(i, i as f32, 0.0), 0.0);
        }
        assert_eq!(tr.active_count(), MAX_TOUCHES);
        let e = tr.moved(s(0, 99.0, 0.0), 1.0).unwrap();
        assert_eq!(e.touches.len(), MAX_TOUCHES);
    }

    #[test]
    fn points_conversion_scales_locations() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 30.0, 60.0).with_radius(8.0), 0.0);
        let e = tr.moved(s(1, 60.0, 90.0).with_radius(8.0), 8.0).unwrap().to_points(2.0);
        let t = e.touches[0];
        assert_eq!((t.location(), t.previous_location(), t.major_radius), ((30.0, 45.0), (15.0, 30.0), Some(4.0)));
        assert_eq!(TouchEvent { timestamp: 0.0, touches: vec![t] }.to_points(0.0).touches[0].location(), (30.0, 45.0));
    }

    #[test]
    fn in_view_subtracts_the_view_origin_after_scaling() {
        let mut tr = TouchTracker::new();
        tr.down(s(1, 300.0, 600.0), 0.0);
        let e = tr.moved(s(1, 330.0, 630.0), 8.0).unwrap().in_view(2.0, 145.0, 295.0);
        let t = e.touches[0];
        assert_eq!((t.location(), t.previous_location()), ((20.0, 20.0), (5.0, 5.0)));
        assert_eq!(t.delta(), (15.0, 15.0));
    }

    #[test]
    fn wire_values_round_trip() {
        for p in [TouchPhase::Began, TouchPhase::Moved, TouchPhase::Stationary, TouchPhase::Ended, TouchPhase::Cancelled] {
            assert_eq!(TouchPhase::from_wire(p as u8), Some(p));
        }
        assert_eq!(TouchPhase::from_wire(5), None);
        for k in [TouchType::Direct, TouchType::Pencil, TouchType::Indirect] {
            assert_eq!(TouchType::from_wire(k as u8), Some(k));
        }
        assert_eq!(TouchType::from_wire(3), None);
    }
}
