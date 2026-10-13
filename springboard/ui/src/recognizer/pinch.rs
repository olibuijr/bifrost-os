//! Pinch: two fingers moving towards or away from each other.

use super::*;

/// Fingers closer than this give no stable scale (points).
const MIN_SEPARATION: f32 = 1.0;

/// Continuous: follows the first two fingers. `scale` is the distance between them relative to when both were down;
/// the gesture begins once the distance changed by `slop` points and ends when one of the two lifts.
#[derive(Debug)]
pub struct Pinch {
    core: Core,
    /// Change of the finger distance before the gesture begins (points).
    pub slop: f32,
    touches: TouchSet,
    pair: TwoTouch,
    start_dist: Option<f32>,
    scale: f32,
    focal: (f32, f32),
    smoother: Option<Smoother>,
    velocity: f32,
}

impl Pinch {
    pub fn new() -> Self {
        Pinch {
            core: Core::new(),
            slop: 6.0,
            touches: TouchSet::default(),
            pair: TwoTouch::default(),
            start_dist: None,
            scale: 1.0,
            focal: (0.0, 0.0),
            smoother: None,
            velocity: 0.0,
        }
    }

    /// Current distance divided by the distance when both fingers came down; 1.0 before that.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Rate of change of the scale in 1/s as of the last sample; positive while the fingers spread.
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Midpoint between the two fingers.
    pub fn focal(&self) -> (f32, f32) {
        self.focal
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.pair = TwoTouch::default();
        self.start_dist = None;
        self.scale = 1.0;
        self.smoother = None;
        self.velocity = 0.0;
    }
}

impl Default for Pinch {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer for Pinch {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        if !self.touches.apply(s) {
            return;
        }
        if s.phase == TouchPhase::Began {
            self.pair.add(s.id);
        }
        let tracked = self.pair.has(s.id);
        let cur = self.core.current();
        if !cur.is_terminal() {
            let mut dist_now = None;
            if let Some((a, b)) = self.pair.positions(&self.touches) {
                let d = dist(a, b);
                dist_now = Some(d);
                self.focal = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                if self.start_dist.is_none() && d >= MIN_SEPARATION {
                    self.start_dist = Some(d);
                }
                if let Some(d0) = self.start_dist {
                    let scale = d / d0;
                    let rest = s.phase == TouchPhase::Ended && scale == self.scale;
                    self.scale = scale;
                    if !rest {
                        self.smoother.get_or_insert_with(|| Smoother::new(scale, s.t)).push(scale, s.t);
                    }
                    if let Some(sm) = &self.smoother {
                        self.velocity = sm.at(s.t);
                    }
                }
            }
            match s.phase {
                TouchPhase::Cancelled if tracked => {
                    self.core.go(if cur == GestureState::Possible { GestureState::Failed } else { GestureState::Cancelled });
                }
                TouchPhase::Ended if tracked => {
                    self.core.go(if cur == GestureState::Possible { GestureState::Failed } else { GestureState::Ended });
                }
                TouchPhase::Began | TouchPhase::Moved => {
                    if let (Some(d), Some(d0)) = (dist_now, self.start_dist) {
                        if cur == GestureState::Possible {
                            if (d - d0).abs() >= self.slop {
                                self.core.go(GestureState::Began);
                            }
                        } else if tracked {
                            self.core.go(GestureState::Changed);
                        }
                    }
                }
                _ => {}
            }
        }
        self.touches.finish(s);
    }

    fn tick(&mut self, _t: f64) {}

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two fingers 100 points apart around x = 150.
    fn down(r: &mut Pinch) {
        r.feed(&TouchSample::began(1, 100.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 200.0, 0.0, 0.0));
    }

    fn spread(r: &mut Pinch, half: f32, t: f64) {
        r.feed(&TouchSample::moved(1, 150.0 - half, 0.0, t));
        r.feed(&TouchSample::moved(2, 150.0 + half, 0.0, t));
    }

    #[test]
    fn two_fingers_alone_do_not_begin() {
        let mut r = Pinch::new();
        down(&mut r);
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.scale(), 1.0);
        assert_eq!(r.focal(), (150.0, 0.0));
    }

    #[test]
    fn spreading_begins_past_the_slop_and_reports_the_scale() {
        let mut r = Pinch::new();
        down(&mut r);
        r.feed(&TouchSample::moved(1, 98.0, 0.0, 0.01));
        assert_eq!(r.state(), GestureState::Possible, "2 points of change is under the slop");
        r.feed(&TouchSample::moved(2, 202.0, 0.0, 0.01));
        r.feed(&TouchSample::moved(1, 95.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Began);
        spread(&mut r, 100.0, 0.03);
        assert_eq!(r.state(), GestureState::Changed);
        assert_eq!(r.scale(), 2.0);
    }

    #[test]
    fn pinching_in_gives_a_scale_below_one() {
        let mut r = Pinch::new();
        down(&mut r);
        spread(&mut r, 25.0, 0.02);
        assert!(r.state().is_active());
        assert_eq!(r.scale(), 0.5);
        assert_eq!(r.focal(), (150.0, 0.0));
    }

    #[test]
    fn velocity_is_the_smoothed_scale_rate() {
        let mut r = Pinch::new();
        down(&mut r);
        // distance 100 -> 200 over half a second in 25 steps: scale 1 -> 2 at 2 per second
        for i in 1..=25 {
            let half = 50.0 + 50.0 * i as f32 / 25.0;
            spread(&mut r, half, 0.02 * i as f64);
        }
        assert!((r.scale() - 2.0).abs() < 1e-4);
        assert!((r.velocity() - 2.0).abs() < 0.1, "got {}", r.velocity());
        r.feed(&TouchSample::ended(1, 50.0, 0.0, 0.51));
        assert_eq!(r.state(), GestureState::Ended);
        assert!((r.velocity() - 2.0).abs() < 0.1, "the release velocity stays");
    }

    #[test]
    fn closing_has_negative_velocity() {
        let mut r = Pinch::new();
        down(&mut r);
        for i in 1..=10 {
            let half = 50.0 - 2.0 * i as f32;
            spread(&mut r, half, 0.02 * i as f64);
        }
        assert!(r.velocity() < 0.0);
        assert!(r.scale() < 1.0);
    }

    #[test]
    fn lifting_a_finger_ends_it() {
        let mut r = Pinch::new();
        down(&mut r);
        spread(&mut r, 80.0, 0.02);
        r.feed(&TouchSample::ended(2, 230.0, 0.0, 0.04));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn lifting_early_fails_and_a_third_finger_is_ignored() {
        let mut r = Pinch::new();
        down(&mut r);
        r.feed(&TouchSample::ended(2, 200.0, 0.0, 0.04));
        assert_eq!(r.state(), GestureState::Failed);

        let mut t = Pinch::new();
        down(&mut t);
        t.feed(&TouchSample::began(3, 400.0, 0.0, 0.01));
        spread(&mut t, 80.0, 0.02);
        assert!(t.state().is_active());
        assert_eq!(t.scale(), 1.6);
        t.feed(&TouchSample::moved(3, 500.0, 0.0, 0.03));
        assert_eq!(t.scale(), 1.6, "the third finger does not change the pinch");
        t.feed(&TouchSample::ended(3, 500.0, 0.0, 0.04));
        assert!(t.state().is_active(), "lifting the third finger does not end it");
    }

    #[test]
    fn cancel_cancels_a_running_pinch() {
        let mut r = Pinch::new();
        down(&mut r);
        spread(&mut r, 80.0, 0.02);
        r.feed(&TouchSample::cancelled(1, 70.0, 0.0, 0.03));
        assert_eq!(r.state(), GestureState::Cancelled);
    }

    #[test]
    fn starts_afresh_for_the_next_pair() {
        let mut r = Pinch::new();
        down(&mut r);
        spread(&mut r, 80.0, 0.02);
        r.feed(&TouchSample::ended(1, 70.0, 0.0, 0.03));
        r.feed(&TouchSample::ended(2, 230.0, 0.0, 0.03));
        r.feed(&TouchSample::began(5, 0.0, 0.0, 1.0));
        r.feed(&TouchSample::began(6, 40.0, 0.0, 1.0));
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.scale(), 1.0);
        r.feed(&TouchSample::moved(6, 80.0, 0.0, 1.05));
        assert_eq!(r.state(), GestureState::Began);
        assert_eq!(r.scale(), 2.0, "relative to the new pair");
    }
}
