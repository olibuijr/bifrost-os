//! Gesture recognizers (`NativeGestureRecognizer`): pointer events in, a state out. Pure state machines driven by the
//! caller's clock, so they are testable without a window. Thresholds are in points and seconds.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Possible,
    Began,
    Changed,
    Ended,
    Failed,
}

/// Movement beyond this many points turns a press into a drag (Golden Gate uses about 4 pt).
pub const SLOP: f32 = 4.0;
pub const LONG_PRESS_SECS: f64 = 0.5;
pub const MULTI_TAP_SECS: f64 = 0.4;

/// Counts taps (single, double, ...) and reports `Ended` when the count is reached.
pub struct Tap {
    pub required: u32,
    count: u32,
    down: Option<(f32, f32, f64)>,
    last_up: f64,
}
impl Tap {
    pub fn new(required: u32) -> Self {
        Tap { required: required.max(1), count: 0, down: None, last_up: f64::NEG_INFINITY }
    }
    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        if t - self.last_up > MULTI_TAP_SECS {
            self.count = 0;
        }
        self.down = Some((x, y, t));
    }
    pub fn moved(&mut self, x: f32, y: f32) -> State {
        match self.down {
            Some((sx, sy, _)) if (x - sx).hypot(y - sy) > SLOP => {
                self.down = None;
                self.count = 0;
                State::Failed
            }
            _ => State::Possible,
        }
    }
    pub fn up(&mut self, t: f64) -> State {
        match self.down.take() {
            Some((_, _, t0)) if t - t0 <= LONG_PRESS_SECS => {
                self.count += 1;
                self.last_up = t;
                if self.count >= self.required {
                    self.count = 0;
                    State::Ended
                } else {
                    State::Possible
                }
            }
            _ => State::Failed,
        }
    }
}

/// Fires `Began` once the pointer has been held still long enough; `Ended` on release after that.
pub struct LongPress {
    down: Option<(f32, f32, f64)>,
    began: bool,
}
impl LongPress {
    pub fn new() -> Self {
        LongPress { down: None, began: false }
    }
    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.down = Some((x, y, t));
        self.began = false;
    }
    /// Call on every frame or timer tick with the current time.
    pub fn tick(&mut self, t: f64) -> State {
        match self.down {
            Some((_, _, t0)) if !self.began && t - t0 >= LONG_PRESS_SECS => {
                self.began = true;
                State::Began
            }
            Some(_) if self.began => State::Changed,
            _ => State::Possible,
        }
    }
    pub fn moved(&mut self, x: f32, y: f32) -> State {
        match self.down {
            Some((sx, sy, _)) if !self.began && (x - sx).hypot(y - sy) > SLOP => {
                self.down = None;
                State::Failed
            }
            _ => State::Possible,
        }
    }
    pub fn up(&mut self) -> State {
        let s = if self.began { State::Ended } else { State::Failed };
        self.down = None;
        self.began = false;
        s
    }
}

/// Drag with translation and release velocity (points per second, from the last 100 ms of movement).
pub struct Pan {
    start: Option<(f32, f32)>,
    active: bool,
    samples: Vec<(f32, f32, f64)>,
}
impl Pan {
    pub fn new() -> Self {
        Pan { start: None, active: false, samples: Vec::new() }
    }
    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.start = Some((x, y));
        self.active = false;
        self.samples.clear();
        self.samples.push((x, y, t));
    }
    pub fn moved(&mut self, x: f32, y: f32, t: f64) -> State {
        let Some((sx, sy)) = self.start else { return State::Possible };
        self.samples.push((x, y, t));
        self.samples.retain(|s| t - s.2 <= 0.1);
        if !self.active {
            if (x - sx).hypot(y - sy) <= SLOP {
                return State::Possible;
            }
            self.active = true;
            return State::Began;
        }
        State::Changed
    }
    pub fn translation(&self) -> (f32, f32) {
        match (self.start, self.samples.last()) {
            (Some((sx, sy)), Some(&(x, y, _))) => (x - sx, y - sy),
            _ => (0.0, 0.0),
        }
    }
    pub fn velocity(&self) -> (f32, f32) {
        match (self.samples.first(), self.samples.last()) {
            (Some(a), Some(b)) if b.2 > a.2 => {
                let dt = (b.2 - a.2) as f32;
                ((b.0 - a.0) / dt, (b.1 - a.1) / dt)
            }
            _ => (0.0, 0.0),
        }
    }
    pub fn up(&mut self) -> State {
        let s = if self.active { State::Ended } else { State::Failed };
        self.start = None;
        self.active = false;
        s
    }
}

/// Two-finger pinch (`NativeMagnificationGestureRecognizer`). The magnification is the change in finger spacing
/// against the spacing when the fingers first landed: 0.0 is no change, 1.0 is twice the spacing.
#[derive(Default)]
pub struct Magnify {
    base: Option<f32>,
    magnification: f32,
    began: bool,
}
impl Magnify {
    /// One frame with both finger positions. Spacing must change by more than `SLOP` points to begin.
    pub fn touches(&mut self, a: (f32, f32), b: (f32, f32)) -> State {
        let spacing = (b.0 - a.0).hypot(b.1 - a.1);
        let base = *self.base.get_or_insert(spacing);
        if base <= 0.0 {
            // Fingers that started on one point have no spacing to scale from.
            return State::Failed;
        }
        self.magnification = spacing / base - 1.0;
        if !self.began {
            if (spacing - base).abs() <= SLOP {
                return State::Possible;
            }
            self.began = true;
            return State::Began;
        }
        State::Changed
    }
    pub fn magnification(&self) -> f32 {
        self.magnification
    }
    /// The fingers lifted: `Ended` if the pinch began, `Failed` if it never got past the slop.
    pub fn up(&mut self) -> State {
        let s = if self.began { State::Ended } else { State::Failed };
        *self = Self::default();
        s
    }
}

/// Two-finger rotation (`NativeRotationGestureRecognizer`). The angle of the line between the fingers, unwrapped at
/// half a turn so the total keeps counting. Angles follow screen coordinates (y down), so positive is clockwise.
#[derive(Default)]
pub struct Rotate {
    last: Option<f32>,
    turned: f32,
    began: bool,
}
/// Turns smaller than this (radians, about 3 degrees) do not begin a rotation.
pub const ROTATION_SLOP: f32 = 0.05;
impl Rotate {
    /// One frame with both finger positions.
    pub fn touches(&mut self, a: (f32, f32), b: (f32, f32)) -> State {
        let angle = (b.1 - a.1).atan2(b.0 - a.0);
        let Some(prev) = self.last.replace(angle) else { return State::Possible };
        let mut step = angle - prev;
        let turn = std::f32::consts::PI;
        if step > turn {
            step -= 2.0 * turn;
        } else if step < -turn {
            step += 2.0 * turn;
        }
        self.turned += step;
        if !self.began {
            if self.turned.abs() <= ROTATION_SLOP {
                return State::Possible;
            }
            self.began = true;
            return State::Began;
        }
        State::Changed
    }
    /// Total turn in radians since the fingers landed.
    pub fn rotation(&self) -> f32 {
        self.turned
    }
    pub fn rotation_degrees(&self) -> f32 {
        self.turned.to_degrees()
    }
    pub fn up(&mut self) -> State {
        let s = if self.began { State::Ended } else { State::Failed };
        *self = Self::default();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_tap_needs_two_quick_taps() {
        let mut t = Tap::new(2);
        t.down(0.0, 0.0, 0.0);
        assert_eq!(t.up(0.05), State::Possible);
        t.down(0.0, 0.0, 0.2);
        assert_eq!(t.up(0.25), State::Ended);
        t.down(0.0, 0.0, 5.0);
        assert_eq!(t.up(5.05), State::Possible); // slow second tap starts over
    }
    #[test]
    fn tap_fails_when_it_drags_or_is_held() {
        let mut t = Tap::new(1);
        t.down(0.0, 0.0, 0.0);
        assert_eq!(t.moved(10.0, 0.0), State::Failed);
        assert_eq!(t.up(0.1), State::Failed);
        t.down(0.0, 0.0, 1.0);
        assert_eq!(t.up(2.0), State::Failed);
    }
    #[test]
    fn long_press_begins_after_hold_and_cancels_on_move() {
        let mut l = LongPress::new();
        l.down(0.0, 0.0, 0.0);
        assert_eq!(l.tick(0.2), State::Possible);
        assert_eq!(l.tick(0.6), State::Began);
        assert_eq!(l.up(), State::Ended);
        l.down(0.0, 0.0, 0.0);
        assert_eq!(l.moved(20.0, 0.0), State::Failed);
        assert_eq!(l.tick(1.0), State::Possible);
    }
    #[test]
    fn pan_slop_translation_and_velocity() {
        let mut p = Pan::new();
        p.down(0.0, 0.0, 0.0);
        assert_eq!(p.moved(2.0, 0.0, 0.01), State::Possible);
        assert_eq!(p.moved(10.0, 0.0, 0.02), State::Began);
        assert_eq!(p.moved(100.0, 0.0, 0.07), State::Changed);
        assert_eq!(p.translation(), (100.0, 0.0));
        let (vx, vy) = p.velocity();
        assert!(vx > 1400.0 && vy == 0.0, "{vx}");
        assert_eq!(p.up(), State::Ended);
    }
    #[test]
    fn pinch_reports_spacing_change_and_ends_when_fingers_lift() {
        let mut m = Magnify::default();
        assert_eq!(m.touches((0., 0.), (100., 0.)), State::Possible);
        assert_eq!(m.touches((0., 0.), (102., 0.)), State::Possible, "2 pt is inside the slop");
        assert_eq!(m.touches((0., 0.), (120., 0.)), State::Began);
        assert!((m.magnification() - 0.2).abs() < 1e-4, "{}", m.magnification());
        assert_eq!(m.touches((0., 0.), (50., 0.)), State::Changed);
        assert!((m.magnification() + 0.5).abs() < 1e-4, "{}", m.magnification());
        assert_eq!(m.up(), State::Ended);
        assert_eq!(m.magnification(), 0.0, "a finished pinch starts from nothing");
    }
    #[test]
    fn pinch_that_never_leaves_the_slop_fails_on_lift() {
        let mut m = Magnify::default();
        m.touches((0., 0.), (100., 0.));
        assert_eq!(m.touches((0., 0.), (103., 0.)), State::Possible);
        assert_eq!(m.up(), State::Failed);
    }
    #[test]
    fn pinch_from_one_point_has_nothing_to_scale() {
        let mut m = Magnify::default();
        assert_eq!(m.touches((5., 5.), (5., 5.)), State::Failed);
        assert_eq!(m.up(), State::Failed);
    }
    #[test]
    fn rotation_begins_past_slop_and_reports_degrees() {
        let mut r = Rotate::default();
        assert_eq!(r.touches((0., 0.), (100., 0.)), State::Possible);
        assert_eq!(r.touches((0., 0.), (0., 100.)), State::Began, "a quarter turn clockwise on screen");
        assert!((r.rotation_degrees() - 90.0).abs() < 1e-2, "{}", r.rotation_degrees());
        assert_eq!(r.touches((0., 0.), (-100., 0.)), State::Changed);
        assert!((r.rotation_degrees() - 180.0).abs() < 1e-2, "{}", r.rotation_degrees());
        assert_eq!(r.up(), State::Ended);
    }
    #[test]
    fn rotation_unwraps_across_the_half_turn() {
        let mut r = Rotate::default();
        r.touches((0., 0.), (-100., 1.));
        // the fingers cross the horizontal: the raw angle jumps from about +pi to about -pi
        assert_eq!(r.touches((0., 0.), (-100., -1.)), State::Possible);
        assert!((r.rotation() - 0.02).abs() < 1e-4, "a 0.02 rad turn, not -2 pi: {}", r.rotation());
    }
    #[test]
    fn rotation_below_the_slop_is_ignored_and_fails_on_lift() {
        let mut r = Rotate::default();
        r.touches((0., 0.), (100., 0.));
        assert_eq!(r.touches((0., 0.), (100., 2.)), State::Possible, "about 0.02 rad");
        assert_eq!(r.up(), State::Failed);
    }
}
