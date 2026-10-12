//! The banner's life: slides in over `SLIDE_MS`, rests `HOLD_MS`, slides
//! out. A finger can pull it up (released far or fast: it leaves; a short
//! pull: it rests again) or tap it.

use super::super::gestures::{ease_out_quartic, progress, SPRING_MS};

/// How long a banner rests fully shown before it leaves.
pub const HOLD_MS: f64 = 5000.0;
/// Slide in and out duration.
pub const SLIDE_MS: f64 = SPRING_MS;
/// A release faster than this (px per ms upwards) throws the banner away.
const FLICK: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    In,
    Hold,
    Out,
    Gone,
}

/// A finger on the banner: where it landed, where it is, its last speed.
#[derive(Clone, Copy, Debug)]
struct Pull {
    y0: f32,
    y: f32,
    t: f64,
    v: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Banner {
    /// the notification shown
    pub id: u64,
    phase: Phase,
    /// when the phase began
    since: f64,
    pull: Option<Pull>,
    /// an alert style banner (Settings > Notifications > Alerts): it rests until the person dismisses it
    persistent: bool,
}

impl Banner {
    pub fn new(id: u64, t: f64) -> Self {
        Banner { id, phase: Phase::In, since: t, pull: None, persistent: false }
    }

    /// The same banner, resting until it is dismissed when `on` (the Alerts style of an app).
    pub fn persistent(mut self, on: bool) -> Self {
        self.persistent = on;
        self
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn gone(&self) -> bool {
        self.phase == Phase::Gone
    }

    /// Sliding in or out: every frame matters.
    pub fn animating(&self) -> bool {
        matches!(self.phase, Phase::In | Phase::Out)
    }

    /// When the rest ends and the slide out starts, unless a finger holds it.
    pub fn next_wake(&self) -> Option<f64> {
        match (self.phase, self.pull) {
            (Phase::Hold, None) if !self.persistent => Some(self.since + HOLD_MS),
            _ => None,
        }
    }

    /// Advances the phases at `t`; true when one ended.
    pub fn tick(&mut self, t: f64) -> bool {
        let mut changed = false;
        loop {
            let (next, len) = match self.phase {
                Phase::In => (Phase::Hold, SLIDE_MS),
                Phase::Hold if self.pull.is_none() && !self.persistent => (Phase::Out, HOLD_MS),
                Phase::Out => (Phase::Gone, SLIDE_MS),
                _ => return changed,
            };
            if t - self.since < len {
                return changed;
            }
            self.since += len;
            self.phase = next;
            changed = true;
        }
    }

    /// 0 (off the top) .. 1 (resting) at `t`.
    pub fn shown(&self, t: f64) -> f32 {
        match self.phase {
            Phase::In => ease_out_quartic(progress(self.since, t, SLIDE_MS)),
            Phase::Hold => 1.0,
            Phase::Out => 1.0 - ease_out_quartic(progress(self.since, t, SLIDE_MS)),
            Phase::Gone => 0.0,
        }
    }

    /// How far the finger has pulled it up, in px (never negative).
    pub fn pull(&self) -> f32 {
        self.pull.map_or(0.0, |p| (p.y0 - p.y).max(0.0))
    }

    pub fn pulling(&self) -> bool {
        self.pull.is_some()
    }

    pub fn down(&mut self, y: f32, t: f64) {
        self.pull = Some(Pull { y0: y, y, t, v: 0.0 });
    }

    pub fn moved(&mut self, y: f32, t: f64) {
        if let Some(p) = &mut self.pull {
            let dt = (t - p.t) as f32;
            if dt > 0.0 {
                p.v = 0.6 * (y - p.y) / dt + 0.4 * p.v;
            }
            p.y = y;
            p.t = t;
        }
    }

    /// Lifts the finger: a tap (moved less than `slop` px) is reported as
    /// true for the caller to act on; a pull past `far` px or a flick up
    /// starts the slide out; otherwise the banner rests again from `t`.
    pub fn up(&mut self, y: f32, t: f64, slop: f32, far: f32) -> bool {
        self.moved(y, t);
        let Some(p) = self.pull.take() else { return false };
        let dy = p.y0 - p.y;
        if dy.abs() < slop {
            return true;
        }
        if dy > far || -p.v > FLICK {
            self.dismiss(t);
        } else {
            self.phase = Phase::Hold;
            self.since = t;
        }
        false
    }

    /// Starts the slide out now (or does nothing if already leaving).
    pub fn dismiss(&mut self, t: f64) {
        if matches!(self.phase, Phase::In | Phase::Hold) {
            self.phase = Phase::Out;
            self.since = t;
            self.pull = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alerts_banner_rests_until_it_is_dismissed() {
        let mut b = Banner::new(3, 0.0).persistent(true);
        assert!(b.tick(SLIDE_MS));
        assert_eq!(b.phase(), Phase::Hold);
        assert_eq!(b.next_wake(), None, "no timer ends a persistent banner");
        assert!(!b.tick(SLIDE_MS + HOLD_MS * 10.0));
        assert_eq!(b.phase(), Phase::Hold);
        b.dismiss(100_000.0);
        assert_eq!(b.phase(), Phase::Out);
    }

    #[test]
    fn slides_in_rests_five_seconds_and_leaves() {
        let mut b = Banner::new(7, 1000.0);
        assert_eq!(b.phase(), Phase::In);
        assert_eq!(b.shown(1000.0), 0.0);
        assert!(b.animating());
        assert!(!b.tick(1000.0 + SLIDE_MS / 2.0));
        assert!(b.shown(1000.0 + SLIDE_MS / 2.0) > 0.5, "ease out: most of the way early");
        assert!(b.tick(1000.0 + SLIDE_MS));
        assert_eq!(b.phase(), Phase::Hold);
        assert_eq!(b.shown(2000.0), 1.0);
        assert!(!b.animating());
        assert_eq!(b.next_wake(), Some(1000.0 + SLIDE_MS + HOLD_MS));
        assert!(!b.tick(1000.0 + SLIDE_MS + HOLD_MS - 1.0));
        assert!(b.tick(1000.0 + SLIDE_MS + HOLD_MS));
        assert_eq!(b.phase(), Phase::Out);
        assert!(b.animating());
        assert!(b.shown(1000.0 + SLIDE_MS + HOLD_MS + SLIDE_MS / 2.0) < 0.5);
        assert!(b.tick(1000.0 + 2.0 * SLIDE_MS + HOLD_MS));
        assert!(b.gone());
        assert_eq!(b.shown(1e9), 0.0);
        assert_eq!(b.next_wake(), None);
    }

    #[test]
    fn a_late_tick_runs_through_every_phase() {
        let mut b = Banner::new(1, 0.0);
        assert!(b.tick(SLIDE_MS + HOLD_MS + SLIDE_MS + 5.0));
        assert!(b.gone());
    }

    #[test]
    fn a_finger_holds_it_and_a_tap_is_reported() {
        let mut b = Banner::new(1, 0.0);
        b.tick(SLIDE_MS);
        b.down(100.0, 500.0);
        assert!(b.pulling());
        assert_eq!(b.next_wake(), None, "no timeout while held");
        assert!(!b.tick(SLIDE_MS + HOLD_MS + 100.0), "held past the rest: still here");
        assert_eq!(b.phase(), Phase::Hold);
        assert!(b.up(103.0, 600.0, 8.0, 40.0), "a tap");
        assert!(!b.pulling());
        assert_eq!(b.phase(), Phase::Hold);
    }

    #[test]
    fn a_short_pull_rests_again_a_far_pull_or_flick_dismisses() {
        let mut b = Banner::new(1, 0.0);
        b.tick(SLIDE_MS);
        b.down(100.0, 1000.0);
        b.moved(80.0, 1200.0);
        assert_eq!(b.pull(), 20.0);
        b.moved(120.0, 1300.0);
        assert_eq!(b.pull(), 0.0, "pulling down does nothing");
        assert!(!b.up(80.0, 1500.0, 8.0, 40.0));
        assert_eq!(b.phase(), Phase::Hold);
        assert_eq!(b.next_wake(), Some(1500.0 + HOLD_MS), "the rest restarts on release");
        b.down(100.0, 2000.0);
        b.moved(40.0, 2500.0);
        assert!(!b.up(40.0, 2600.0, 8.0, 40.0));
        assert_eq!(b.phase(), Phase::Out);
        assert!(b.animating());
        let mut f = Banner::new(2, 0.0);
        f.tick(SLIDE_MS);
        f.down(100.0, 1000.0);
        f.moved(90.0, 1005.0);
        f.moved(75.0, 1010.0);
        assert!(!f.up(60.0, 1015.0, 8.0, 400.0));
        assert_eq!(f.phase(), Phase::Out, "a flick leaves even when short");
        f.dismiss(2000.0);
        assert_eq!(f.phase(), Phase::Out, "dismiss never restarts a slide out");
    }
}
