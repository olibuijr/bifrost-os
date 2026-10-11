//! Touch state machines for the system gestures: a vertical drag that
//! reveals or hides an overlay (Control Center) and the swipe up from the
//! home indicator that shrinks the app and ends at home, in the switcher or
//! back in the app. Times are milliseconds, distances pixels.

/// Duration of the settle animations, an bifrost-like spring ease-out.
pub const SPRING_MS: f64 = 350.0;
/// A release faster than this (px per ms) decides by direction alone.
const FLICK: f32 = 0.9;
/// A release slower than this (px per ms) counts as a pause.
const SLOW: f32 = 0.15;
/// No movement for this long before the release means the finger paused.
const PAUSE_MS: f64 = 120.0;
/// Resistance past the ends of a drag.
const RUBBER: f32 = 0.35;

/// Spring-like ease-out: quick start, soft landing.
pub fn ease_out_quartic(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(4)
}

/// Progress of an animation that started at `start` and runs `ms`.
pub fn progress(start: f64, t: f64, ms: f64) -> f32 {
    ((t - start) / ms).clamp(0.0, 1.0) as f32
}

/// Rubber band: linear inside 0..=1, resisted beyond.
pub fn rubber(p: f32) -> f32 {
    if p < 0.0 {
        p * RUBBER
    } else if p > 1.0 {
        1.0 + (p - 1.0) * RUBBER
    } else {
        p
    }
}

/// Smoothed vertical finger velocity in px per ms (negative = up).
#[derive(Clone, Copy, Debug)]
pub struct Velocity {
    y: f32,
    t: f64,
    v: f32,
}

impl Velocity {
    pub fn new(y: f32, t: f64) -> Self {
        Velocity { y, t, v: 0.0 }
    }

    /// Blends in a sample; after a pause the old velocity no longer counts.
    pub fn push(&mut self, y: f32, t: f64) {
        let dt = t - self.t;
        if dt > 0.0 {
            let now = (y - self.y) / dt as f32;
            self.v = if dt > PAUSE_MS { now } else { 0.6 * now + 0.4 * self.v };
            self.y = y;
            self.t = t;
        }
    }

    /// The velocity at release time `t`: zero if the finger rested first.
    pub fn at(&self, t: f64) -> f32 {
        if t - self.t > PAUSE_MS {
            0.0
        } else {
            self.v
        }
    }
}

/// A vertical drag that moves an overlay between hidden (0) and shown (1).
/// `from` is the overlay's state when the finger landed, `range` the travel
/// in px for a full reveal; dragging down reveals.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    y0: f32,
    y: f32,
    from: f32,
    range: f32,
    vel: Velocity,
}

impl Drag {
    pub fn new(y: f32, t: f64, from: f32, range: f32) -> Self {
        Drag { y0: y, y, from, range, vel: Velocity::new(y, t) }
    }

    pub fn moved(&mut self, y: f32, t: f64) {
        self.y = y;
        self.vel.push(y, t);
    }

    /// Finger travel since touch-down, positive downwards.
    pub fn travel(&self) -> f32 {
        self.y - self.y0
    }

    /// The overlay's position under the finger, rubber-banded past the ends.
    pub fn progress(&self) -> f32 {
        rubber(self.from + self.travel() / self.range)
    }

    /// Whether the overlay should settle shown when released at `t`.
    pub fn release(&self, t: f64) -> bool {
        let v = self.vel.at(t);
        if v.abs() > FLICK {
            return v > 0.0;
        }
        let threshold = if self.from < 0.5 { 0.35 } else { 0.65 };
        self.progress() > threshold
    }
}

/// Where a go-home swipe ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HomeOutcome {
    /// too short or moving back down: the app springs back
    Cancel,
    /// a flick or a steady swipe up
    Home,
    /// lifted and paused
    Switcher,
}

/// The swipe up from the home indicator; the app card follows the finger.
#[derive(Clone, Copy, Debug)]
pub struct HomeGesture {
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    vel: Velocity,
}

impl HomeGesture {
    pub fn new(x: f32, y: f32, t: f64) -> Self {
        HomeGesture { x0: x, y0: y, x, y, vel: Velocity::new(y, t) }
    }

    pub fn moved(&mut self, x: f32, y: f32, t: f64) {
        self.x = x;
        self.y = y;
        self.vel.push(y, t);
    }

    /// How far the finger has risen, in px (never negative).
    pub fn lift(&self) -> f32 {
        (self.y0 - self.y).max(0.0)
    }

    /// Horizontal drift since touch-down.
    pub fn drift(&self) -> f32 {
        self.x - self.x0
    }

    /// Shrink progress 0..=1 over `range` px of lift.
    pub fn progress(&self, range: f32) -> f32 {
        (self.lift() / range).clamp(0.0, 1.0)
    }

    /// Decides the outcome on release at `t`; `min_lift` is the px a swipe
    /// must travel before it counts at all.
    pub fn release(&self, t: f64, min_lift: f32) -> HomeOutcome {
        let v = self.vel.at(t);
        if v < -FLICK {
            return HomeOutcome::Home;
        }
        if self.lift() < min_lift {
            return HomeOutcome::Cancel;
        }
        if v.abs() < SLOW {
            HomeOutcome::Switcher
        } else if v < 0.0 {
            HomeOutcome::Home
        } else {
            HomeOutcome::Cancel
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moves `g` from y0 to y1 in `steps` over `ms`, starting at `t0`.
    fn swipe(g: &mut HomeGesture, y0: f32, y1: f32, t0: f64, ms: f64, steps: usize) -> f64 {
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            g.moved(600.0, y0 + (y1 - y0) * k, t0 + ms * i as f64 / steps as f64);
        }
        t0 + ms
    }

    #[test]
    fn ease_out_is_monotonic_and_bounded() {
        assert_eq!(ease_out_quartic(0.0), 0.0);
        assert_eq!(ease_out_quartic(1.0), 1.0);
        assert_eq!(ease_out_quartic(2.0), 1.0);
        let mut last = 0.0;
        for i in 1..=10 {
            let v = ease_out_quartic(i as f32 / 10.0);
            assert!(v > last);
            last = v;
        }
        assert!(ease_out_quartic(0.3) > 0.6, "most of the motion happens early");
        assert_eq!(progress(1000.0, 1175.0, SPRING_MS), 0.5);
    }

    #[test]
    fn rubber_band_resists_past_the_ends() {
        assert_eq!(rubber(0.5), 0.5);
        assert!(rubber(-1.0) > -0.5 && rubber(-1.0) < 0.0);
        assert!(rubber(2.0) < 1.5 && rubber(2.0) > 1.0);
    }

    #[test]
    fn velocity_smooths_and_zeroes_after_a_pause() {
        let mut v = Velocity::new(100.0, 0.0);
        v.push(90.0, 10.0);
        v.push(80.0, 20.0);
        assert!(v.at(25.0) < -0.5);
        assert_eq!(v.at(20.0 + PAUSE_MS + 1.0), 0.0, "resting finger has no velocity");
        v.push(80.0, 20.0 + PAUSE_MS + 1.0);
        assert_eq!(v.at(20.0 + PAUSE_MS + 1.0), 0.0, "a sample after the pause forgets the old speed");
    }

    #[test]
    fn drag_reveals_and_hides_with_rubber_band() {
        let mut d = Drag::new(10.0, 0.0, 0.0, 400.0);
        d.moved(210.0, 100.0);
        assert_eq!(d.progress(), 0.5);
        assert_eq!(d.travel(), 200.0);
        d.moved(610.0, 200.0);
        assert!(d.progress() > 1.0 && d.progress() < 1.3);
        let mut back = Drag::new(500.0, 0.0, 1.0, 400.0);
        back.moved(300.0, 100.0);
        assert_eq!(back.progress(), 0.5);
        back.moved(0.0, 200.0);
        assert!(back.progress() < 0.0 && back.progress() > -0.3);
    }

    #[test]
    fn drag_release_uses_flick_then_position() {
        // slow, short: stays hidden
        let mut d = Drag::new(0.0, 0.0, 0.0, 400.0);
        d.moved(50.0, 500.0);
        assert!(!d.release(500.0));
        // slow but far enough: shows
        d.moved(200.0, 1500.0);
        assert!(d.release(1500.0));
        // fast flick down from the start: shows even when short
        let mut f = Drag::new(0.0, 0.0, 0.0, 400.0);
        f.moved(20.0, 10.0);
        f.moved(50.0, 20.0);
        assert!(f.release(20.0));
        // from shown, a short slow drag up keeps it shown; a flick up hides
        let mut up = Drag::new(600.0, 0.0, 1.0, 400.0);
        up.moved(550.0, 500.0);
        assert!(up.release(500.0));
        up.moved(300.0, 510.0);
        assert!(!up.release(510.0));
        // a paused far drag up hides by position
        let mut slow = Drag::new(600.0, 0.0, 1.0, 400.0);
        slow.moved(300.0, 1000.0);
        assert!(!slow.release(1000.0 + PAUSE_MS * 2.0));
    }

    #[test]
    fn home_gesture_tracks_lift_drift_and_progress() {
        let mut g = HomeGesture::new(600.0, 1900.0, 0.0);
        g.moved(650.0, 1500.0, 100.0);
        assert_eq!(g.lift(), 400.0);
        assert_eq!(g.drift(), 50.0);
        assert_eq!(g.progress(800.0), 0.5);
        g.moved(650.0, 1950.0, 200.0);
        assert_eq!(g.lift(), 0.0, "below the start counts as no lift");
        assert_eq!(g.progress(800.0), 0.0);
    }

    #[test]
    fn flick_up_goes_home_even_when_short() {
        let mut g = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut g, 1900.0, 1820.0, 0.0, 40.0, 4);
        assert_eq!(g.release(t, 100.0), HomeOutcome::Home);
    }

    #[test]
    fn short_slow_swipe_cancels() {
        let mut g = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut g, 1900.0, 1860.0, 0.0, 400.0, 4);
        assert_eq!(g.release(t, 100.0), HomeOutcome::Cancel);
    }

    #[test]
    fn lift_and_pause_opens_the_switcher() {
        let mut g = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut g, 1900.0, 1400.0, 0.0, 300.0, 10);
        assert_eq!(g.release(t + 300.0, 100.0), HomeOutcome::Switcher, "rested before lifting");
        let mut crawl = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut crawl, 1900.0, 1400.0, 0.0, 5000.0, 50);
        assert_eq!(crawl.release(t, 100.0), HomeOutcome::Switcher, "creeping up counts as a pause");
    }

    #[test]
    fn steady_swipe_up_goes_home_and_returning_down_cancels() {
        let mut g = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut g, 1900.0, 1400.0, 0.0, 1000.0, 10);
        assert_eq!(g.release(t, 100.0), HomeOutcome::Home);
        let mut back = HomeGesture::new(600.0, 1900.0, 0.0);
        let t = swipe(&mut back, 1900.0, 1400.0, 0.0, 300.0, 10);
        let t = swipe(&mut back, 1400.0, 1600.0, t, 200.0, 5);
        assert_eq!(back.release(t, 100.0), HomeOutcome::Cancel);
    }
}
