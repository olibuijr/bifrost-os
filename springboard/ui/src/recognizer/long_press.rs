//! Long press: fingers held down for a while without moving much.

use super::*;

/// Continuous: `Possible -> Began` once the hold time passes, `Changed` while the fingers move, `Ended` on release.
#[derive(Debug)]
pub struct LongPress {
    core: Core,
    /// Hold time before the gesture begins (seconds).
    pub min_duration: f64,
    /// Movement allowed before it begins (points); afterwards the fingers may travel freely.
    pub allowable_movement: f32,
    pub touches_required: usize,
    touches: TouchSet,
    armed_at: Option<f64>,
    location: (f32, f32),
}

impl LongPress {
    pub fn new() -> Self {
        LongPress {
            core: Core::new(),
            min_duration: 0.5,
            allowable_movement: 10.0,
            touches_required: 1,
            touches: TouchSet::default(),
            armed_at: None,
            location: (0.0, 0.0),
        }
    }

    pub fn with_touches(mut self, n: usize) -> Self {
        self.touches_required = n.max(1);
        self
    }

    /// Centroid of the fingers.
    pub fn location(&self) -> (f32, f32) {
        self.location
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.armed_at = None;
    }

    fn fail(&mut self) {
        self.core.go(GestureState::Failed);
    }
}

impl Default for LongPress {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer for LongPress {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        self.tick(s.t);
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        if !self.touches.apply(s) {
            return;
        }
        self.location = self.touches.centroid();
        let cur = self.core.current();
        if !cur.is_terminal() {
            let waiting = cur == GestureState::Possible;
            match s.phase {
                TouchPhase::Began => {
                    let n = self.touches.len();
                    if waiting {
                        if n > self.touches_required {
                            self.fail();
                        } else if n == self.touches_required {
                            self.armed_at = Some(s.t);
                        }
                    }
                }
                TouchPhase::Moved => {
                    if waiting {
                        let limit = self.allowable_movement;
                        if self.touches.iter().any(|t| t.travel() > limit) {
                            self.fail();
                        }
                    } else {
                        self.core.go(GestureState::Changed);
                    }
                }
                TouchPhase::Ended => {
                    if waiting {
                        self.fail();
                    } else if self.touches.len() <= self.touches_required {
                        self.core.go(GestureState::Ended);
                    }
                }
                TouchPhase::Cancelled => {
                    if waiting {
                        self.fail();
                    } else {
                        self.core.go(GestureState::Cancelled);
                    }
                }
            }
        }
        self.touches.finish(s);
    }

    fn tick(&mut self, t: f64) {
        if self.core.current() != GestureState::Possible {
            return;
        }
        if let Some(armed) = self.armed_at {
            if self.touches.len() == self.touches_required && t - armed >= self.min_duration {
                self.core.go(GestureState::Began);
            }
        }
    }

    fn deadline(&self) -> Option<f64> {
        if self.core.current() != GestureState::Possible {
            return None;
        }
        self.armed_at.map(|a| a + self.min_duration)
    }

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizer::testutil::*;

    #[test]
    fn begins_after_the_hold_time_on_tick() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 20.0, 30.0, 0.0));
        assert_eq!(r.deadline(), Some(0.5));
        r.tick(0.3);
        assert_eq!(r.state(), GestureState::Possible);
        r.tick(0.5);
        assert_eq!(r.state(), GestureState::Began);
        assert_eq!(r.location(), (20.0, 30.0));
        assert_eq!(r.deadline(), None);
        r.feed(&TouchSample::ended(1, 20.0, 30.0, 0.9));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn a_late_sample_starts_the_press_at_its_deadline_first() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 50.0, 0.0, 0.7));
        assert_eq!(r.state(), GestureState::Changed, "it began at 0.5, the later move is a change");
    }

    #[test]
    fn moving_before_the_time_fails() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 5.0, 0.0, 0.1));
        assert_eq!(r.state(), GestureState::Possible);
        r.feed(&TouchSample::moved(1, 15.0, 0.0, 0.2));
        assert_eq!(r.state(), GestureState::Failed);
        r.tick(1.0);
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn releasing_early_fails() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::ended(1, 0.0, 0.0, 0.2));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn after_it_began_the_fingers_may_travel_far() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.tick(0.6);
        run(&mut r, &drag(1, (0.0, 0.0), (200.0, 0.0), 0.6, 1.0, 4));
        assert_eq!(r.state(), GestureState::Changed);
        assert_eq!(r.location(), (200.0, 0.0));
    }

    #[test]
    fn custom_duration_and_movement() {
        let mut r = LongPress::new();
        r.min_duration = 1.0;
        r.allowable_movement = 30.0;
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 25.0, 0.0, 0.2));
        r.tick(0.9);
        assert_eq!(r.state(), GestureState::Possible);
        r.tick(1.0);
        assert_eq!(r.state(), GestureState::Began);
    }

    #[test]
    fn two_finger_press_waits_for_the_second_finger() {
        let mut r = LongPress::new().with_touches(2);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.tick(0.6);
        assert_eq!(r.state(), GestureState::Possible, "one finger is not enough");
        r.feed(&TouchSample::began(2, 40.0, 0.0, 0.7));
        r.tick(1.1);
        assert_eq!(r.state(), GestureState::Possible, "the time counts from the second finger");
        r.tick(1.25);
        assert_eq!(r.state(), GestureState::Began);
        assert_eq!(r.location(), (20.0, 0.0));
        r.feed(&TouchSample::ended(2, 40.0, 0.0, 1.5));
        assert_eq!(r.state(), GestureState::Ended, "lifting a finger ends it");
    }

    #[test]
    fn an_extra_finger_fails_a_pending_press() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 40.0, 0.0, 0.1));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn cancel_during_the_press_cancels() {
        let mut r = LongPress::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.tick(0.6);
        r.feed(&TouchSample::cancelled(1, 0.0, 0.0, 0.7));
        assert_eq!(r.state(), GestureState::Cancelled);

        let mut p = LongPress::new();
        p.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        p.feed(&TouchSample::cancelled(1, 0.0, 0.0, 0.1));
        assert_eq!(p.state(), GestureState::Failed);
    }

    #[test]
    fn works_with_a_pointer_and_restarts_on_the_next_press() {
        let mut p = PointerAdapter::new();
        let mut r = LongPress::new();
        r.feed(&p.press(1.0, 1.0, 0.0));
        r.tick(0.6);
        assert_eq!(r.state(), GestureState::Began);
        r.feed(&p.release(1.0, 1.0, 0.7).unwrap());
        assert_eq!(r.state(), GestureState::Ended);
        r.feed(&p.press(1.0, 1.0, 2.0));
        assert_eq!(r.state(), GestureState::Possible);
        r.tick(2.6);
        assert_eq!(r.state(), GestureState::Began);
    }
}
