//! Pan: one or more fingers dragging.

use super::edge::{Edge, EdgeRule};
use super::*;

enum Verdict {
    Wait,
    Begin,
    Fail,
}

/// Continuous: `Possible -> Began` once the fingers moved `slop` points, `Changed` while they move, `Ended` when
/// fewer than `min_touches` remain.
#[derive(Debug)]
pub struct Pan {
    pub(super) core: Core,
    pub min_touches: usize,
    pub max_touches: usize,
    /// Travel of the centroid before the gesture begins (points).
    pub slop: f32,
    touches: TouchSet,
    translation: (f32, f32),
    /// The touch count and centroid the translation was last measured against.
    prev: Option<(usize, (f32, f32))>,
    velocity: (f32, f32),
    location: (f32, f32),
    pub(super) edge: Option<EdgeRule>,
}

impl Pan {
    pub fn new() -> Self {
        Pan {
            core: Core::new(),
            min_touches: 1,
            max_touches: usize::MAX,
            slop: 10.0,
            touches: TouchSet::default(),
            translation: (0.0, 0.0),
            prev: None,
            velocity: (0.0, 0.0),
            location: (0.0, 0.0),
            edge: None,
        }
    }

    pub fn with_touches(mut self, min: usize, max: usize) -> Self {
        self.min_touches = min.max(1);
        self.max_touches = max.max(self.min_touches);
        self
    }

    /// Travel of the centroid since the touches came down; adding or lifting a finger does not jump it.
    pub fn translation(&self) -> (f32, f32) {
        self.translation
    }

    /// Replaces the translation, for callers that apply it incrementally and want it to restart from zero.
    pub fn set_translation(&mut self, x: f32, y: f32) {
        self.translation = (x, y);
    }

    /// Mean velocity of the fingers in points per second as of the last sample; a gesture that ended keeps the
    /// release velocity.
    pub fn velocity(&self) -> (f32, f32) {
        self.velocity
    }

    /// Velocity as of `t`: zero for fingers that have rested.
    pub fn velocity_at(&self, t: f64) -> (f32, f32) {
        if self.touches.is_empty() {
            self.velocity
        } else {
            self.touches.velocity_at(t)
        }
    }

    /// Centroid of the fingers.
    pub fn location(&self) -> (f32, f32) {
        self.location
    }

    pub fn touch_count(&self) -> usize {
        self.touches.len()
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.translation = (0.0, 0.0);
        self.prev = None;
        self.velocity = (0.0, 0.0);
        if let Some(rule) = &mut self.edge {
            rule.hit = None;
        }
    }

    fn in_range(&self, n: usize) -> bool {
        n >= self.min_touches && n <= self.max_touches
    }

    fn update_translation(&mut self) {
        let n = self.touches.len();
        if self.in_range(n) {
            let c = self.touches.centroid();
            if let Some((pn, pc)) = self.prev {
                if pn == n {
                    self.translation.0 += c.0 - pc.0;
                    self.translation.1 += c.1 - pc.1;
                }
            }
            self.prev = Some((n, c));
        } else {
            self.prev = None;
        }
    }

    fn rebase(&mut self) {
        let n = self.touches.len();
        self.prev = if n > 0 && self.in_range(n) { Some((n, self.touches.centroid())) } else { None };
    }

    fn verdict(&self) -> Verdict {
        let (tx, ty) = self.translation;
        match &self.edge {
            None => {
                if tx.hypot(ty) >= self.slop {
                    Verdict::Begin
                } else {
                    Verdict::Wait
                }
            }
            Some(rule) => match rule.hit {
                None => Verdict::Fail,
                Some(edge) => {
                    let (inward, lateral) = edge.split(tx, ty);
                    if inward <= -self.slop {
                        Verdict::Fail
                    } else if inward >= self.slop && inward >= lateral {
                        Verdict::Begin
                    } else if lateral >= self.slop && lateral > inward {
                        Verdict::Fail
                    } else {
                        Verdict::Wait
                    }
                }
            },
        }
    }

    /// The edge a screen-edge pan started at.
    pub(super) fn edge_hit(&self) -> Option<Edge> {
        self.edge.as_ref().and_then(|r| r.hit)
    }

    fn fail(&mut self) {
        self.core.go(GestureState::Failed);
    }
}

impl Default for Pan {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer for Pan {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        let was_empty = self.touches.is_empty();
        if !self.touches.apply(s) {
            return;
        }
        self.location = self.touches.centroid();
        self.velocity = self.touches.velocity_at(s.t);
        self.update_translation();

        let n = self.touches.len();
        let ending = matches!(s.phase, TouchPhase::Ended | TouchPhase::Cancelled);
        let remaining = if ending { n - 1 } else { n };
        if was_empty && s.phase == TouchPhase::Began && self.core.current() == GestureState::Possible {
            if let Some(rule) = &mut self.edge {
                rule.hit = rule.edge_at((s.x, s.y));
                if rule.hit.is_none() {
                    self.core.go(GestureState::Failed);
                }
            }
        }

        match self.core.current() {
            GestureState::Possible => {
                if s.phase == TouchPhase::Cancelled || n > self.max_touches {
                    self.fail();
                } else {
                    if self.in_range(n) {
                        match self.verdict() {
                            Verdict::Begin => self.core.go(GestureState::Began),
                            Verdict::Fail => self.fail(),
                            Verdict::Wait => {}
                        }
                    }
                    if s.phase == TouchPhase::Ended {
                        match self.core.current() {
                            GestureState::Began => {
                                if remaining < self.min_touches {
                                    self.core.go(GestureState::Ended);
                                } else {
                                    self.core.go(GestureState::Changed);
                                }
                            }
                            GestureState::Possible => {
                                if remaining == 0 {
                                    self.fail();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            GestureState::Began | GestureState::Changed => match s.phase {
                TouchPhase::Cancelled => self.core.go(GestureState::Cancelled),
                TouchPhase::Ended => {
                    if remaining < self.min_touches {
                        self.core.go(GestureState::Ended);
                    } else {
                        self.core.go(GestureState::Changed);
                    }
                }
                TouchPhase::Began | TouchPhase::Moved => {
                    if n > self.max_touches {
                        self.core.go(GestureState::Ended);
                    } else if n >= self.min_touches {
                        self.core.go(GestureState::Changed);
                    }
                }
            },
            _ => {}
        }

        self.touches.finish(s);
        if ending {
            self.rebase();
        }
    }

    fn tick(&mut self, _t: f64) {}

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizer::testutil::*;

    /// Down at the origin and a steady drag to (`dx`, 0) over `secs`, 25 samples.
    fn steady(r: &mut Pan, dx: f32, secs: f64) {
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        run(r, &drag(1, (0.0, 0.0), (dx, 0.0), 0.0, secs, 25));
    }

    #[test]
    fn begins_after_the_slop_and_tracks_translation() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 100.0, 100.0, 0.0));
        r.feed(&TouchSample::moved(1, 105.0, 100.0, 0.01));
        assert_eq!(r.state(), GestureState::Possible, "under the slop");
        r.feed(&TouchSample::moved(1, 112.0, 100.0, 0.02));
        assert_eq!(r.state(), GestureState::Began);
        assert_eq!(r.translation(), (12.0, 0.0));
        r.feed(&TouchSample::moved(1, 140.0, 130.0, 0.03));
        assert_eq!(r.state(), GestureState::Changed);
        assert_eq!(r.translation(), (40.0, 30.0));
        assert_eq!(r.location(), (140.0, 130.0));
        r.feed(&TouchSample::ended(1, 140.0, 130.0, 0.04));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn release_without_enough_movement_fails() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 3.0, 0.0, 0.01));
        r.feed(&TouchSample::ended(1, 3.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn coarse_input_that_jumps_in_the_release_sample_still_pans() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::ended(1, 60.0, 0.0, 0.05));
        assert_eq!(r.state(), GestureState::Ended);
        assert_eq!(r.translation(), (60.0, 0.0));
    }

    #[test]
    fn velocity_follows_a_steady_drag() {
        let mut r = Pan::new();
        steady(&mut r, 300.0, 0.5);
        let (vx, vy) = r.velocity();
        assert!((vx - 600.0).abs() < 6.0, "600 points per second, got {vx}");
        assert_eq!(vy, 0.0);
        assert!((r.velocity_at(0.51).0 - 600.0).abs() < 6.0);
        assert_eq!(r.velocity_at(0.5 + PAUSE_SECS + 0.1), 0.0, "rested fingers have no velocity");
    }

    #[test]
    fn release_velocity_is_kept_after_the_gesture_ends() {
        let mut r = Pan::new();
        steady(&mut r, 300.0, 0.5);
        r.feed(&TouchSample::ended(1, 300.0, 0.0, 0.51));
        assert_eq!(r.state(), GestureState::Ended);
        assert!((r.velocity().0 - 600.0).abs() < 6.0);
        assert!((r.velocity_at(5.0).0 - 600.0).abs() < 6.0, "the recorded release velocity");
    }

    #[test]
    fn resting_before_release_gives_zero_velocity() {
        let mut r = Pan::new();
        steady(&mut r, 300.0, 0.5);
        r.feed(&TouchSample::ended(1, 300.0, 0.0, 0.9));
        assert_eq!(r.velocity(), (0.0, 0.0));
    }

    #[test]
    fn two_finger_pan_waits_for_two_fingers_and_ends_when_one_lifts() {
        let mut r = Pan::new().with_touches(2, 2);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 50.0, 0.0, 0.05));
        assert_eq!(r.state(), GestureState::Possible, "one finger is not enough");
        r.feed(&TouchSample::began(2, 100.0, 0.0, 0.06));
        r.feed(&TouchSample::moved(1, 70.0, 0.0, 0.08));
        r.feed(&TouchSample::moved(2, 120.0, 0.0, 0.08));
        assert!(r.state().is_active());
        assert_eq!(r.translation(), (20.0, 0.0), "only movement of both fingers counts");
        r.feed(&TouchSample::ended(2, 120.0, 0.0, 0.1));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn a_third_finger_fails_a_two_finger_pan_that_has_not_begun() {
        let mut r = Pan::new().with_touches(1, 2);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 50.0, 0.0, 0.0));
        r.feed(&TouchSample::began(3, 90.0, 0.0, 0.0));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn a_third_finger_ends_a_pan_that_has_begun() {
        let mut r = Pan::new().with_touches(1, 2);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 30.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Began);
        r.feed(&TouchSample::began(2, 50.0, 0.0, 0.03));
        assert_eq!(r.state(), GestureState::Changed);
        r.feed(&TouchSample::began(3, 90.0, 0.0, 0.04));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn adding_or_lifting_a_finger_does_not_jump_the_translation() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 30.0, 0.0, 0.02));
        assert_eq!(r.translation(), (30.0, 0.0));
        r.feed(&TouchSample::began(2, 230.0, 0.0, 0.03));
        assert_eq!(r.translation(), (30.0, 0.0), "the centroid moved but the translation did not");
        r.feed(&TouchSample::moved(2, 250.0, 0.0, 0.04));
        assert_eq!(r.translation(), (40.0, 0.0), "the centroid follows half of the second finger");
        r.feed(&TouchSample::ended(2, 250.0, 0.0, 0.05));
        assert_eq!(r.translation(), (40.0, 0.0));
        assert_eq!(r.state(), GestureState::Changed, "one finger remains");
        r.feed(&TouchSample::moved(1, 40.0, 0.0, 0.06));
        assert_eq!(r.translation(), (50.0, 0.0));
    }

    #[test]
    fn set_translation_restarts_the_measurement() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 30.0, 0.0, 0.02));
        r.set_translation(0.0, 0.0);
        r.feed(&TouchSample::moved(1, 35.0, 0.0, 0.03));
        assert_eq!(r.translation(), (5.0, 0.0));
    }

    #[test]
    fn cancel_cancels_a_running_pan_and_fails_a_pending_one() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 30.0, 0.0, 0.02));
        r.feed(&TouchSample::cancelled(1, 30.0, 0.0, 0.03));
        assert_eq!(r.state(), GestureState::Cancelled);
        let mut p = Pan::new();
        p.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        p.feed(&TouchSample::cancelled(1, 0.0, 0.0, 0.03));
        assert_eq!(p.state(), GestureState::Failed);
    }

    #[test]
    fn restarts_with_a_clean_translation_for_the_next_touch() {
        let mut r = Pan::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 50.0, 0.0, 0.02));
        r.feed(&TouchSample::ended(1, 50.0, 0.0, 0.03));
        r.feed(&TouchSample::began(2, 300.0, 300.0, 1.0));
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.translation(), (0.0, 0.0));
        assert_eq!(r.touch_count(), 1);
    }

    #[test]
    fn works_with_a_pointer_drag() {
        let mut p = PointerAdapter::new();
        let mut r = Pan::new();
        r.feed(&p.press(0.0, 0.0, 0.0));
        r.feed(&p.motion(0.0, 40.0, 0.05).unwrap());
        assert_eq!(r.state(), GestureState::Began);
        r.feed(&p.release(0.0, 40.0, 0.06).unwrap());
        assert_eq!(r.state(), GestureState::Ended);
        assert_eq!(r.translation(), (0.0, 40.0));
    }
}
