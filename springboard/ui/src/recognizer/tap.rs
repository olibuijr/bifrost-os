//! Tap: one or more quick touch-and-release sequences with a given number of fingers.

use super::*;

/// Fingers of a multi-touch tap must land within this many seconds of the first.
const LAND_WINDOW_SECS: f64 = 0.25;
/// A repeated tap must land within this many points of the first tap.
const REPEAT_RADIUS: f32 = 30.0;

/// Recognizes `taps_required` taps of `touches_required` fingers. Discrete: `Possible -> Recognized | Failed`.
#[derive(Debug)]
pub struct Tap {
    core: Core,
    pub taps_required: u32,
    pub touches_required: usize,
    /// A finger moving farther than this from where it landed fails the tap (points).
    pub max_movement: f32,
    /// A tap held longer than this fails (seconds).
    pub max_hold: f64,
    /// The next tap of a multi-tap must start within this long after the last one ended (seconds).
    pub max_gap: f64,
    touches: TouchSet,
    taps: u32,
    first_down: Option<f64>,
    peak: usize,
    lifted: usize,
    anchor: Option<(f32, f32)>,
    lift_loc: Option<(f32, f32)>,
    deadline: Option<f64>,
    location: (f32, f32),
}

impl Tap {
    pub fn new(taps_required: u32, touches_required: usize) -> Self {
        Tap {
            core: Core::new(),
            taps_required: taps_required.max(1),
            touches_required: touches_required.max(1),
            max_movement: 10.0,
            max_hold: 0.5,
            max_gap: 0.35,
            touches: TouchSet::default(),
            taps: 0,
            first_down: None,
            peak: 0,
            lifted: 0,
            anchor: None,
            lift_loc: None,
            deadline: None,
            location: (0.0, 0.0),
        }
    }

    /// Where the fingers were when they lifted (the centroid), valid once recognized.
    pub fn location(&self) -> (f32, f32) {
        self.location
    }

    /// Taps completed so far in the current sequence.
    pub fn taps_done(&self) -> u32 {
        self.taps
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.taps = 0;
        self.first_down = None;
        self.peak = 0;
        self.lifted = 0;
        self.anchor = None;
        self.lift_loc = None;
        self.deadline = None;
    }

    fn fail(&mut self) {
        self.core.go(GestureState::Failed);
    }
}

impl Recognizer for Tap {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        self.tick(s.t);
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        let was_empty = self.touches.is_empty();
        if !self.touches.apply(s) {
            return;
        }
        if !self.core.current().is_terminal() {
            match s.phase {
                TouchPhase::Began => {
                    if was_empty {
                        if let (true, Some(a)) = (self.taps > 0, self.anchor) {
                            if dist(a, (s.x, s.y)) > REPEAT_RADIUS {
                                self.fail();
                            }
                        }
                        self.deadline = None;
                        self.first_down = Some(s.t);
                        self.peak = 0;
                        self.lifted = 0;
                        self.lift_loc = None;
                        if self.taps == 0 {
                            self.anchor = Some((s.x, s.y));
                        }
                    }
                    let n = self.touches.len();
                    if n > self.touches_required {
                        self.fail();
                    } else if self.first_down.is_some_and(|f| s.t - f > LAND_WINDOW_SECS) {
                        self.fail();
                    }
                    self.peak = self.peak.max(n);
                }
                TouchPhase::Moved => {
                    if self.touches.get(s.id).is_some_and(|t| t.travel() > self.max_movement) {
                        self.fail();
                    }
                }
                TouchPhase::Ended => {
                    if self.touches.get(s.id).is_some_and(|t| t.travel() > self.max_movement) {
                        self.fail();
                    } else if self.peak < self.touches_required {
                        self.fail();
                    } else {
                        if self.lifted == 0 {
                            self.lift_loc = Some(self.touches.centroid());
                        }
                        self.lifted += 1;
                        if self.touches.len() == 1 {
                            let held_ok = self.first_down.is_some_and(|f| s.t - f <= self.max_hold);
                            if held_ok {
                                self.taps += 1;
                                if self.taps >= self.taps_required {
                                    self.location = self.lift_loc.unwrap_or((s.x, s.y));
                                    self.core.go(GestureState::Recognized);
                                } else {
                                    self.deadline = Some(s.t + self.max_gap);
                                    self.first_down = None;
                                }
                            } else {
                                self.fail();
                            }
                        }
                    }
                }
                TouchPhase::Cancelled => self.fail(),
            }
        }
        self.touches.finish(s);
    }

    fn tick(&mut self, t: f64) {
        if self.core.current().is_terminal() {
            return;
        }
        if self.deadline.is_some_and(|d| t > d) {
            self.fail();
            return;
        }
        if !self.touches.is_empty() && self.first_down.is_some_and(|f| t - f > self.max_hold) {
            self.fail();
        }
    }

    fn deadline(&self) -> Option<f64> {
        if self.core.current().is_terminal() {
            return None;
        }
        if let Some(d) = self.deadline {
            return Some(d);
        }
        if !self.touches.is_empty() {
            return self.first_down.map(|f| f + self.max_hold);
        }
        None
    }

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap_at(r: &mut Tap, id: u64, x: f32, y: f32, t0: f64, hold: f64) {
        r.feed(&TouchSample::began(id, x, y, t0));
        r.feed(&TouchSample::ended(id, x, y, t0 + hold));
    }

    #[test]
    fn single_tap_is_recognized_on_release() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::began(1, 50.0, 60.0, 0.0));
        assert_eq!(r.state(), GestureState::Possible);
        r.feed(&TouchSample::ended(1, 50.0, 60.0, 0.1));
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.location(), (50.0, 60.0));
    }

    #[test]
    fn small_wobble_still_taps_but_a_drag_fails() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 6.0, 0.0, 0.03));
        r.feed(&TouchSample::ended(1, 6.0, 0.0, 0.06));
        assert_eq!(r.state(), GestureState::Recognized);

        let mut d = Tap::new(1, 1);
        d.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        d.feed(&TouchSample::moved(1, 30.0, 0.0, 0.03));
        assert_eq!(d.state(), GestureState::Failed);
        d.feed(&TouchSample::ended(1, 30.0, 0.0, 0.06));
        assert_eq!(d.state(), GestureState::Failed, "a failed tap stays failed");
    }

    #[test]
    fn holding_too_long_fails_on_tick_and_on_release() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        assert_eq!(r.deadline(), Some(0.5));
        r.tick(0.4);
        assert_eq!(r.state(), GestureState::Possible);
        r.tick(0.6);
        assert_eq!(r.state(), GestureState::Failed);

        let mut q = Tap::new(1, 1);
        tap_at(&mut q, 1, 0.0, 0.0, 0.0, 0.8);
        assert_eq!(q.state(), GestureState::Failed);
    }

    #[test]
    fn double_tap_needs_both_taps_in_time_and_place() {
        let mut r = Tap::new(2, 1);
        tap_at(&mut r, 1, 10.0, 10.0, 0.0, 0.05);
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.taps_done(), 1);
        assert!((r.deadline().unwrap() - 0.40).abs() < 1e-9);
        tap_at(&mut r, 2, 14.0, 12.0, 0.2, 0.05);
        assert_eq!(r.state(), GestureState::Recognized);

        let mut slow = Tap::new(2, 1);
        tap_at(&mut slow, 1, 10.0, 10.0, 0.0, 0.05);
        slow.tick(0.6);
        assert_eq!(slow.state(), GestureState::Failed, "the second tap never came");

        let mut late = Tap::new(2, 1);
        tap_at(&mut late, 1, 10.0, 10.0, 0.0, 0.05);
        tap_at(&mut late, 2, 10.0, 10.0, 0.6, 0.05);
        assert_eq!(late.state(), GestureState::Possible, "a late tap starts a new sequence");
        assert_eq!(late.taps_done(), 1);

        let mut far = Tap::new(2, 1);
        tap_at(&mut far, 1, 10.0, 10.0, 0.0, 0.05);
        tap_at(&mut far, 2, 200.0, 10.0, 0.2, 0.05);
        assert_eq!(far.state(), GestureState::Failed, "the second tap landed elsewhere");
    }

    #[test]
    fn triple_tap_counts_three() {
        let mut r = Tap::new(3, 1);
        tap_at(&mut r, 1, 0.0, 0.0, 0.0, 0.05);
        tap_at(&mut r, 2, 0.0, 0.0, 0.2, 0.05);
        assert_eq!(r.state(), GestureState::Possible);
        tap_at(&mut r, 3, 0.0, 0.0, 0.4, 0.05);
        assert_eq!(r.state(), GestureState::Recognized);
    }

    #[test]
    fn two_finger_tap_needs_both_fingers() {
        let mut r = Tap::new(1, 2);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 40.0, 0.0, 0.05));
        r.feed(&TouchSample::ended(1, 0.0, 0.0, 0.1));
        assert_eq!(r.state(), GestureState::Possible);
        r.feed(&TouchSample::ended(2, 40.0, 0.0, 0.12));
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.location(), (20.0, 0.0), "the centroid of both fingers");

        let mut one = Tap::new(1, 2);
        tap_at(&mut one, 1, 0.0, 0.0, 0.0, 0.05);
        assert_eq!(one.state(), GestureState::Failed, "one finger is not a two finger tap");
    }

    #[test]
    fn extra_finger_or_late_finger_fails() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 40.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Failed);

        let mut late = Tap::new(1, 2);
        late.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        late.feed(&TouchSample::began(2, 40.0, 0.0, 0.4));
        assert_eq!(late.state(), GestureState::Failed, "fingers did not land together");
    }

    #[test]
    fn cancelled_touch_fails_and_next_touch_starts_afresh() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::cancelled(1, 0.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Failed);
        tap_at(&mut r, 2, 5.0, 5.0, 1.0, 0.05);
        assert_eq!(r.state(), GestureState::Recognized, "a new touch resets the recognizer");
        tap_at(&mut r, 3, 5.0, 5.0, 2.0, 0.05);
        assert_eq!(r.state(), GestureState::Recognized);
    }

    #[test]
    fn works_with_a_pointer() {
        let mut p = PointerAdapter::new();
        let mut r = Tap::new(1, 1);
        r.feed(&p.press(3.0, 4.0, 0.0));
        r.feed(&p.release(3.0, 4.0, 0.08).unwrap());
        assert_eq!(r.state(), GestureState::Recognized);
    }

    #[test]
    fn samples_of_unknown_touches_are_ignored() {
        let mut r = Tap::new(1, 1);
        r.feed(&TouchSample::ended(7, 0.0, 0.0, 0.0));
        assert_eq!(r.state(), GestureState::Possible);
    }
}
