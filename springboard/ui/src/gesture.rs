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
}
