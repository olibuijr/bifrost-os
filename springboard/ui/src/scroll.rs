//! Vertical scrolling with bifrost feel: drag with slop, fling with exponential
//! deceleration, rubber-banding past the ends and a spring back.

use super::rubber_band;

const SLOP: f32 = 12.0;
/// NativeScrollView's normal deceleration rate (velocity kept per ms)
const FRICTION: f32 = 0.998;
const STOP_V: f32 = 0.02;
const MIN_FLING: f32 = 0.15;
/// a finger that rested this long before lifting does not fling
const PAUSE_MS: f64 = 60.0;
/// the window the release velocity is measured over, like bifrost
const VELOCITY_MS: f64 = 100.0;
/// the bounce spring (critically damped, per ms): ~400 ms to settle
const SPRING_W: f32 = 0.012;
/// rubber band: how far past the edge a pull of `x` px goes, `(1 - 1/(x*0.55/D + 1)) * D`
const RUBBER_D: f32 = 1000.0;
const SAMPLES: usize = 16;

/// Where a revealed rectangle is placed in the viewport: the edge or the middle of the target meets that viewport place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// the rectangle's top edge meets the viewport's top edge
    Leading,
    /// the rectangle's middle meets the viewport's middle
    Center,
    /// the rectangle's bottom edge meets the viewport's bottom edge
    Trailing,
}

/// The offset that places `top..top + height` (document points) at `anchor` in a viewport `viewport_h` tall, for a
/// document `content_h` tall. The offset is clamped to the scrollable range `0..=content_h - viewport_h`, so a target
/// near an edge lands at the edge. Non-finite input gives offset zero.
pub fn offset_for_rect(top: f32, height: f32, viewport_h: f32, content_h: f32, anchor: Anchor) -> f32 {
    if ![top, height, viewport_h, content_h].iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    let max = (content_h - viewport_h).max(0.0);
    let wanted = match anchor {
        Anchor::Leading => top,
        Anchor::Center => top + height / 2.0 - viewport_h / 2.0,
        Anchor::Trailing => top + height - viewport_h,
    };
    wanted.clamp(0.0, max)
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    y0: f32,
    offset0: f32,
    dragging: bool,
    /// the finger position the drag is measured from (past the slop, so the content does not jump)
    anchor: f32,
    samples: [(f64, f32); SAMPLES],
    n: usize,
}

impl Touch {
    fn push(&mut self, t: f64, y: f32) {
        if self.n == SAMPLES {
            self.samples.copy_within(1.., 0);
            self.n -= 1;
        }
        self.samples[self.n] = (t, y);
        self.n += 1;
    }

    /// Finger velocity in px/ms over the last `VELOCITY_MS` (positive = finger moving down).
    fn velocity(&self) -> f32 {
        if self.n < 2 {
            return 0.0;
        }
        let (t1, y1) = self.samples[self.n - 1];
        let first = self.samples[..self.n].iter().copied().find(|&(t, _)| t1 - t <= VELOCITY_MS).unwrap_or(self.samples[self.n - 2]);
        let (t0, y0) = if first.0 < t1 { first } else { self.samples[self.n - 2] };
        let dt = (t1 - t0) as f32;
        if dt <= 0.0 { 0.0 } else { (y1 - y0) / dt }
    }

    fn last_t(&self) -> f64 {
        if self.n == 0 { f64::NEG_INFINITY } else { self.samples[self.n - 1].0 }
    }
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Fling { v: f32, t: f64 },
    /// a critically damped spring to `to`, from offset `x0` past it with velocity `v0` at `start`
    Spring { x0: f32, v0: f32, to: f32, start: f64 },
}

#[derive(Debug)]
pub struct Scroller {
    offset: f32,
    max: f32,
    touch: Option<Touch>,
    motion: Option<Motion>,
}

fn rubber(x: f32) -> f32 {
    rubber_band(x, RUBBER_D)
}

impl Scroller {
    pub fn new() -> Self {
        Scroller { offset: 0.0, max: 0.0, touch: None, motion: None }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn animating(&self) -> bool {
        self.motion.is_some()
    }

    pub fn dragging(&self) -> bool {
        self.touch.is_some_and(|t| t.dragging)
    }

    pub fn set_max(&mut self, max: f32) {
        self.max = max.max(0.0);
        if self.touch.is_none() && self.motion.is_none() {
            self.offset = self.offset.clamp(0.0, self.max);
        }
    }

    pub fn jump_top(&mut self) {
        self.jump_to(0.0);
    }

    pub fn jump_to(&mut self, offset: f32) {
        self.offset = offset.clamp(0.0, self.max);
        self.motion = None;
    }

    /// `scrollWheel:`: the wheel moved the content `dy` (the content's travel in the units of `set_max`, so a wheel
    /// turned down is negative and shows what is below). Stops any fling and clamps to `0..=max`. Not taken while a
    /// touch is down, or when there is nothing to scroll; true when the offset changed.
    pub fn wheel(&mut self, dy: f32) -> bool {
        if self.touch.is_some() || self.max <= 0.0 {
            return false;
        }
        let before = self.offset;
        self.jump_to(self.offset - dy);
        self.offset != before
    }

    pub fn down(&mut self, y: f32, t: f64) {
        // a touch during a fling catches the content where it is
        self.motion = None;
        let mut tc = Touch { y0: y, offset0: self.offset, dragging: false, anchor: y, samples: [(0.0, 0.0); SAMPLES], n: 0 };
        tc.push(t, y);
        self.touch = Some(tc);
    }

    pub fn moved(&mut self, y: f32, t: f64) {
        let max = self.max;
        let Some(tc) = self.touch.as_mut() else { return };
        if !tc.dragging && (y - tc.y0).abs() > SLOP {
            tc.dragging = true;
            tc.anchor = tc.y0 + SLOP.copysign(y - tc.y0);
            tc.n = 0;
        }
        tc.push(t, y);
        if !tc.dragging {
            return;
        }
        let raw = tc.offset0 - (y - tc.anchor);
        self.offset = if raw < 0.0 {
            -rubber(-raw)
        } else if raw > max {
            max + rubber(raw - max)
        } else {
            raw
        };
    }

    pub fn up(&mut self, y: f32, t: f64) -> bool {
        let paused = self.touch.is_some_and(|tc| t - tc.last_t() > PAUSE_MS);
        self.moved(y, t);
        let Some(tc) = self.touch.take() else { return false };
        if !tc.dragging {
            return true;
        }
        let fv = tc.velocity();
        let v = if paused || fv.abs() < MIN_FLING { 0.0 } else { -fv };
        self.motion = Some(match self.spring_target() {
            // released past the edge: spring back (a fling outwards adds to the overshoot)
            Some(to) => Motion::Spring { x0: self.offset - to, v0: v, to, start: t },
            None => Motion::Fling { v, t },
        });
        self.tick(t);
        false
    }

    fn spring_target(&self) -> Option<f32> {
        if self.offset < 0.0 {
            Some(0.0)
        } else if self.offset > self.max {
            Some(self.max)
        } else {
            None
        }
    }

    pub fn tick(&mut self, t: f64) -> bool {
        match self.motion {
            Some(Motion::Fling { v, t: t0 }) => {
                let dt = (t - t0).max(0.0) as f32;
                let decay = FRICTION.powf(dt);
                self.offset += v * (decay - 1.0) / FRICTION.ln();
                let v = v * decay;
                self.motion = match self.spring_target() {
                    // hitting the edge at speed: overshoot with that speed and spring back, as bifrost does
                    Some(to) => Some(Motion::Spring { x0: self.offset - to, v0: v, to, start: t }),
                    None if v.abs() < STOP_V => None,
                    None => Some(Motion::Fling { v, t }),
                };
            }
            Some(Motion::Spring { x0, v0, to, start }) => {
                let dt = (t - start).max(0.0) as f32;
                let w = SPRING_W;
                let e = (-w * dt).exp();
                let x = (x0 + (v0 + w * x0) * dt) * e;
                let v = (v0 - w * (v0 + w * x0) * dt) * e;
                self.offset = to + x;
                if x.abs() < 0.5 && v.abs() < 0.01 {
                    self.offset = to;
                    self.motion = None;
                }
            }
            None => {}
        }
        self.motion.is_some()
    }
}

impl Default for Scroller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_wheel_moves_the_offset_by_its_travel_within_the_range() {
        let mut s = super::Scroller::new();
        assert!(!s.wheel(-30.0), "nothing to scroll");
        s.set_max(100.0);
        assert!(s.wheel(-30.0));
        assert_eq!(s.offset(), 30.0);
        assert!(s.wheel(-500.0));
        assert_eq!(s.offset(), 100.0);
        assert!(!s.wheel(-1.0), "already at the end");
        s.down(10.0, 0.0);
        assert!(!s.wheel(50.0), "a touch that is down keeps the content");
    }

    use super::*;

    fn settle(s: &mut Scroller, mut t: f64) {
        for _ in 0..600 {
            t += 16.0;
            if !s.tick(t) {
                break;
            }
        }
    }

    #[test]
    fn tap_without_drag() {
        let mut s = Scroller::new();
        s.set_max(1000.0);
        s.down(500.0, 0.0);
        s.moved(505.0, 10.0);
        assert!(!s.dragging());
        assert!(s.up(505.0, 20.0));
        assert_eq!(s.offset(), 0.0);
    }

    #[test]
    fn slow_drag_tracks_the_finger() {
        let mut s = Scroller::new();
        s.set_max(1000.0);
        s.down(800.0, 0.0);
        for i in 1..=20 {
            s.moved(800.0 - i as f32 * 10.0, i as f64 * 50.0);
        }
        assert!(s.dragging());
        // the content follows the finger 1:1 from the point the drag was recognised (no jump)
        assert_eq!(s.offset(), 200.0 - SLOP);
        assert!(!s.up(600.0, 1100.0));
        settle(&mut s, 1100.0);
        assert_eq!(s.offset(), 200.0 - SLOP, "a slow release stops where it is");
    }

    #[test]
    fn fling_coasts_and_stops_in_bounds() {
        let mut s = Scroller::new();
        s.set_max(5000.0);
        s.down(900.0, 0.0);
        s.moved(860.0, 16.0);
        s.moved(800.0, 32.0);
        s.moved(720.0, 48.0);
        s.up(700.0, 56.0);
        assert!(s.animating());
        let after_drag = s.offset();
        settle(&mut s, 56.0);
        assert!(s.offset() > after_drag + 300.0, "coasted on: {}", s.offset());
        assert!(s.offset() <= 5000.0);
        assert!(!s.animating());
    }

    #[test]
    fn rubber_bands_and_springs_back() {
        let mut s = Scroller::new();
        s.set_max(300.0);
        s.down(100.0, 0.0);
        s.moved(400.0, 100.0);
        assert!(s.offset() < 0.0 && s.offset() > -300.0);
        s.up(400.0, 1000.0);
        settle(&mut s, 1000.0);
        assert_eq!(s.offset(), 0.0);
        s.down(900.0, 2000.0);
        s.moved(100.0, 2100.0);
        assert!(s.offset() > 300.0);
        s.up(100.0, 3000.0);
        settle(&mut s, 3000.0);
        assert_eq!(s.offset(), 300.0);
    }

    #[test]
    fn fling_past_the_end_springs_to_the_edge() {
        let mut s = Scroller::new();
        s.set_max(100.0);
        s.down(900.0, 0.0);
        s.moved(700.0, 16.0);
        s.moved(500.0, 32.0);
        s.up(300.0, 48.0);
        let mut peak: f32 = 0.0;
        let mut t = 48.0;
        for _ in 0..300 {
            t += 16.0;
            if !s.tick(t) {
                break;
            }
            peak = peak.max(s.offset());
        }
        assert!(peak > 100.0, "overshot");
        assert_eq!(s.offset(), 100.0);
    }

    #[test]
    fn jump_to_clamps_and_stops_motion() {
        let mut s = Scroller::new();
        s.set_max(300.0);
        s.down(900.0, 0.0);
        s.moved(700.0, 16.0);
        s.up(500.0, 32.0);
        assert!(s.animating());
        s.jump_to(120.0);
        assert!(!s.animating());
        assert_eq!(s.offset(), 120.0);
        s.jump_to(-5.0);
        assert_eq!(s.offset(), 0.0);
        s.jump_to(900.0);
        assert_eq!(s.offset(), 300.0);
    }

    #[test]
    fn shrinking_content_clamps() {
        let mut s = Scroller::new();
        s.set_max(1000.0);
        s.down(900.0, 0.0);
        s.moved(100.0, 500.0);
        s.up(100.0, 1500.0);
        settle(&mut s, 1500.0);
        assert_eq!(s.offset(), 800.0 - SLOP);
        s.set_max(100.0);
        assert_eq!(s.offset(), 100.0);
        s.jump_to(40.0);
        assert_eq!(s.offset(), 40.0);
        s.jump_to(900.0);
        assert_eq!(s.offset(), 100.0, "clamped to the end");
        s.jump_top();
        assert_eq!(s.offset(), 0.0);
    }
}
