//! Runs several recognizers on the same touches and decides which one wins.
//!
//! * Failure requirements: `require_failure(a, b)` holds back everything `a` would publish until `b` has failed.
//!   If `b` is recognized instead, `a` fails. This is how a single tap waits for a double tap, or a pan for a
//!   screen-edge pan.
//! * Exclusivity: when a recognizer begins (or a discrete one is recognized), every other recognizer that has
//!   not begun fails and every one in progress is cancelled, unless the pair was allowed to run simultaneously
//!   with `allow_simultaneous`. Among recognizers that begin on the same sample the one added first wins.
//! * Time: callers feed samples and call [`GestureArbiter::tick`] from a timer; [`GestureArbiter::next_deadline`]
//!   says when a tick can change something.

use super::*;

/// A state change of one recognizer, as the observer sees it. `id` is what [`GestureArbiter::add`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GestureEvent {
    pub id: usize,
    pub state: GestureState,
}

struct Slot {
    r: Box<dyn Recognizer>,
    requires: Vec<usize>,
    simultaneous: Vec<usize>,
    enabled: bool,
}

/// A set of recognizers working on the same touches.
#[derive(Default)]
pub struct GestureArbiter {
    slots: Vec<Slot>,
    /// Touches currently down.
    active: Vec<u64>,
}

impl GestureArbiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a recognizer; earlier ones win ties. Returns its id.
    pub fn add<R: Recognizer>(&mut self, mut r: R) -> usize {
        r.core_mut().set_logging(true);
        self.slots.push(Slot { r: Box::new(r), requires: Vec::new(), simultaneous: Vec::new(), enabled: true });
        self.slots.len() - 1
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// `who` publishes nothing until `of` has failed; it fails if `of` is recognized.
    pub fn require_failure(&mut self, who: usize, of: usize) {
        assert!(who < self.slots.len() && of < self.slots.len() && who != of, "bad recognizer ids");
        if !self.slots[who].requires.contains(&of) {
            self.slots[who].requires.push(of);
        }
    }

    /// Lets `a` and `b` be recognized at the same time.
    pub fn allow_simultaneous(&mut self, a: usize, b: usize) {
        assert!(a < self.slots.len() && b < self.slots.len() && a != b, "bad recognizer ids");
        if !self.slots[a].simultaneous.contains(&b) {
            self.slots[a].simultaneous.push(b);
        }
        if !self.slots[b].simultaneous.contains(&a) {
            self.slots[b].simultaneous.push(a);
        }
    }

    /// Turns a recognizer on or off. Turning one off ends what it was doing.
    pub fn set_enabled(&mut self, id: usize, enabled: bool) -> Vec<GestureEvent> {
        if self.slots[id].enabled && !enabled {
            // Only a gesture in progress is reported as cancelled; one that never showed anything just goes away.
            if self.slots[id].r.core().state().is_active() {
                self.slots[id].r.core_mut().force_fail();
            }
            self.slots[id].r.reset();
        }
        self.slots[id].enabled = enabled;
        self.resolve();
        self.flush()
    }

    pub fn is_enabled(&self, id: usize) -> bool {
        self.slots[id].enabled
    }

    /// What the observer sees for recognizer `id`.
    pub fn state(&self, id: usize) -> GestureState {
        self.slots[id].r.core().state()
    }

    /// The recognizer `id` as its concrete type, to read its translation, scale and so on.
    pub fn get<T: Recognizer>(&self, id: usize) -> Option<&T> {
        self.slots.get(id)?.r.as_any().downcast_ref::<T>()
    }

    pub fn get_mut<T: Recognizer>(&mut self, id: usize) -> Option<&mut T> {
        self.slots.get_mut(id)?.r.as_any_mut().downcast_mut::<T>()
    }

    /// The earliest time at which a tick changes a recognizer.
    pub fn next_deadline(&self) -> Option<f64> {
        self.slots
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.r.deadline())
            .min_by(|a, b| a.total_cmp(b))
    }

    /// Consumes one touch sample and returns the state changes it caused.
    pub fn feed(&mut self, s: &TouchSample) -> Vec<GestureEvent> {
        self.tick_all(s.t);
        self.resolve();
        if s.phase == TouchPhase::Began && self.active.is_empty() {
            self.start_cycle();
        }
        if s.phase == TouchPhase::Began && !self.active.contains(&s.id) {
            self.active.push(s.id);
        }
        for i in 0..self.slots.len() {
            if !self.slots[i].enabled {
                continue;
            }
            let gated = self.is_gated(i);
            let slot = &mut self.slots[i];
            slot.r.core_mut().set_gated(gated);
            slot.r.feed(s);
            self.resolve();
        }
        if matches!(s.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.active.retain(|&id| id != s.id);
        }
        self.flush()
    }

    /// Lets time pass; returns the state changes it caused.
    pub fn tick(&mut self, t: f64) -> Vec<GestureEvent> {
        self.tick_all(t);
        self.resolve();
        self.flush()
    }

    /// The system took all touches away: everything in progress is cancelled and every recognizer starts over.
    pub fn cancel_all(&mut self) -> Vec<GestureEvent> {
        for slot in &mut self.slots {
            if slot.r.core().state().is_active() {
                slot.r.core_mut().force_fail();
            }
            slot.r.reset();
        }
        self.active.clear();
        self.flush()
    }

    /// Recognizers that finished the last touch cycle start over.
    fn start_cycle(&mut self) {
        for slot in &mut self.slots {
            if slot.r.core().busy() {
                continue;
            }
            let state = slot.r.core().state();
            if state.is_active() {
                slot.r.core_mut().force_fail();
            }
            if state != GestureState::Possible {
                slot.r.reset();
            }
        }
    }

    fn tick_all(&mut self, t: f64) {
        for i in 0..self.slots.len() {
            if !self.slots[i].enabled {
                continue;
            }
            let gated = self.is_gated(i);
            let slot = &mut self.slots[i];
            slot.r.core_mut().set_gated(gated);
            slot.r.tick(t);
            // Settle before the next one ticks, so two that come due together do not both begin.
            self.resolve();
        }
    }

    /// Whether `i` still waits for a recognizer it requires to fail.
    fn is_gated(&self, i: usize) -> bool {
        self.slots[i].requires.iter().any(|&j| {
            let other = &self.slots[j];
            other.enabled && !other.r.core().state().is_failure()
        })
    }

    fn resolve(&mut self) {
        let n = self.slots.len();
        // Every pass either changes a recognizer for good or stops, so a few passes per recognizer suffice.
        for _ in 0..(4 * n + 4) {
            let mut changed = false;
            for i in 0..n {
                if !self.slots[i].enabled {
                    continue;
                }
                let gated = self.is_gated(i);
                let required_won = self.slots[i].requires.iter().any(|&j| {
                    let other = &self.slots[j];
                    other.enabled && other.r.core().state().is_success()
                });
                let core = self.slots[i].r.core_mut();
                core.set_gated(gated);
                if core.state() == GestureState::Possible {
                    if required_won && core.current() != GestureState::Failed {
                        core.force_fail();
                        changed = true;
                    } else if !gated && core.busy() {
                        core.release();
                        changed = true;
                    }
                }
                if self.slots[i].r.core_mut().take_began() {
                    for k in 0..n {
                        if k == i || !self.slots[k].enabled || self.slots[i].simultaneous.contains(&k) {
                            continue;
                        }
                        let other = self.slots[k].r.core_mut();
                        let before = other.state();
                        other.force_fail();
                        if other.state() != before {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn flush(&mut self) -> Vec<GestureEvent> {
        let mut out = Vec::new();
        for (id, slot) in self.slots.iter_mut().enumerate() {
            for state in slot.r.core_mut().take_log() {
                out.push(GestureEvent { id, state });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states_of(events: &[GestureEvent], id: usize) -> Vec<GestureState> {
        events.iter().filter(|e| e.id == id).map(|e| e.state).collect()
    }

    fn feed_all(a: &mut GestureArbiter, samples: &[TouchSample]) -> Vec<GestureEvent> {
        samples.iter().flat_map(|s| a.feed(s)).collect()
    }

    fn tap(id: u64, x: f32, y: f32, t0: f64) -> [TouchSample; 2] {
        [TouchSample::began(id, x, y, t0), TouchSample::ended(id, x, y, t0 + 0.05)]
    }

    fn tap_pair() -> (GestureArbiter, usize, usize) {
        let mut a = GestureArbiter::new();
        let single = a.add(Tap::new(1, 1));
        let double = a.add(Tap::new(2, 1));
        a.require_failure(single, double);
        (a, single, double)
    }

    #[test]
    fn a_lone_tap_is_recognized_alone() {
        let mut a = GestureArbiter::new();
        let t = a.add(Tap::new(1, 1));
        let ev = feed_all(&mut a, &tap(1, 5.0, 5.0, 0.0));
        assert_eq!(ev, vec![GestureEvent { id: t, state: GestureState::Recognized }]);
        assert_eq!(a.get::<Tap>(t).unwrap().location(), (5.0, 5.0));
    }

    #[test]
    fn single_tap_waits_for_the_double_tap_to_fail() {
        let (mut a, single, double) = tap_pair();
        let ev = feed_all(&mut a, &tap(1, 10.0, 10.0, 0.0));
        assert!(ev.is_empty(), "the single tap is held back: {ev:?}");
        assert_eq!(a.state(single), GestureState::Possible);
        assert!((a.next_deadline().unwrap() - 0.4).abs() < 1e-9);
        assert!(a.tick(0.3).is_empty());
        let ev = a.tick(0.5);
        assert_eq!(states_of(&ev, double), vec![GestureState::Failed]);
        assert_eq!(states_of(&ev, single), vec![GestureState::Recognized]);
        assert_eq!(a.state(single), GestureState::Recognized);
    }

    #[test]
    fn single_tap_is_also_released_by_a_late_second_touch() {
        let (mut a, single, double) = tap_pair();
        feed_all(&mut a, &tap(1, 10.0, 10.0, 0.0));
        let ev = a.feed(&TouchSample::began(2, 10.0, 10.0, 0.6));
        assert_eq!(states_of(&ev, single), vec![GestureState::Recognized]);
        assert_eq!(states_of(&ev, double), vec![GestureState::Failed]);
    }

    #[test]
    fn double_tap_wins_and_the_single_tap_fails() {
        let (mut a, single, double) = tap_pair();
        let mut ev = feed_all(&mut a, &tap(1, 10.0, 10.0, 0.0));
        ev.extend(feed_all(&mut a, &tap(2, 12.0, 10.0, 0.2)));
        assert_eq!(states_of(&ev, double), vec![GestureState::Recognized]);
        assert_eq!(states_of(&ev, single), vec![GestureState::Failed]);
        assert_eq!(a.next_deadline(), None);
    }

    #[test]
    fn a_new_cycle_starts_clean_after_a_double_tap() {
        let (mut a, single, double) = tap_pair();
        feed_all(&mut a, &tap(1, 10.0, 10.0, 0.0));
        feed_all(&mut a, &tap(2, 10.0, 10.0, 0.2));
        feed_all(&mut a, &tap(3, 10.0, 10.0, 2.0));
        a.tick(3.0);
        assert_eq!(a.state(single), GestureState::Recognized, "a lone tap again recognizes as a single tap");
        assert_eq!(a.state(double), GestureState::Failed);
    }

    #[test]
    fn a_long_hold_is_neither_tap() {
        let (mut a, single, double) = tap_pair();
        a.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        let ev = a.tick(0.7);
        assert_eq!(states_of(&ev, single), vec![GestureState::Failed]);
        assert_eq!(states_of(&ev, double), vec![GestureState::Failed]);
    }

    #[test]
    fn pan_waits_for_the_screen_edge_pan_and_the_edge_pan_wins() {
        let mut a = GestureArbiter::new();
        let edge = a.add(ScreenEdgePan::new(EdgeMask::LEFT, (800.0, 600.0)));
        let pan = a.add(Pan::new());
        a.require_failure(pan, edge);
        let ev = feed_all(
            &mut a,
            &[TouchSample::began(1, 5.0, 300.0, 0.0), TouchSample::moved(1, 40.0, 300.0, 0.05)],
        );
        assert_eq!(states_of(&ev, edge), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Failed], "never began, just failed");
        let ev = a.feed(&TouchSample::moved(1, 80.0, 300.0, 0.1));
        assert_eq!(ev, vec![GestureEvent { id: edge, state: GestureState::Changed }]);
    }

    #[test]
    fn pan_starts_at_once_when_the_touch_is_not_at_an_edge() {
        let mut a = GestureArbiter::new();
        let edge = a.add(ScreenEdgePan::new(EdgeMask::LEFT, (800.0, 600.0)));
        let pan = a.add(Pan::new());
        a.require_failure(pan, edge);
        let ev = feed_all(
            &mut a,
            &[TouchSample::began(1, 300.0, 300.0, 0.0), TouchSample::moved(1, 340.0, 300.0, 0.05)],
        );
        assert_eq!(states_of(&ev, edge), vec![GestureState::Failed]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began]);
    }

    #[test]
    fn the_wait_does_not_depend_on_the_order_the_recognizers_were_added() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        let edge = a.add(ScreenEdgePan::new(EdgeMask::LEFT, (800.0, 600.0)));
        a.require_failure(pan, edge);
        let ev = feed_all(
            &mut a,
            &[TouchSample::began(1, 300.0, 300.0, 0.0), TouchSample::moved(1, 340.0, 300.0, 0.05)],
        );
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, edge), vec![GestureState::Failed]);
    }

    #[test]
    fn a_held_continuous_gesture_replays_its_whole_story() {
        // The pan requires a swipe to fail; the swipe is too slow and fails when the finger lifts.
        let mut a = GestureArbiter::new();
        let swipe = a.add(Swipe::new(DirectionMask::RIGHT, 1));
        let pan = a.add(Pan::new());
        a.require_failure(pan, swipe);
        let ev = feed_all(
            &mut a,
            &[
                TouchSample::began(1, 0.0, 0.0, 0.0),
                TouchSample::moved(1, 20.0, 0.0, 0.5),
                TouchSample::moved(1, 40.0, 0.0, 0.9),
            ],
        );
        assert!(states_of(&ev, pan).is_empty(), "held while the swipe is still possible");
        let ev = a.feed(&TouchSample::ended(1, 40.0, 0.0, 0.95));
        assert_eq!(states_of(&ev, swipe), vec![GestureState::Failed]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began, GestureState::Changed, GestureState::Ended]);
        assert_eq!(a.get::<Pan>(pan).unwrap().translation(), (40.0, 0.0));
    }

    #[test]
    fn long_press_wins_over_a_pan_that_has_not_moved() {
        let mut a = GestureArbiter::new();
        let lp = a.add(LongPress::new());
        let pan = a.add(Pan::new());
        a.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        assert_eq!(a.next_deadline(), Some(0.5));
        let ev = a.tick(0.6);
        assert_eq!(states_of(&ev, lp), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Failed]);
        let ev = a.feed(&TouchSample::moved(1, 100.0, 0.0, 0.7));
        assert_eq!(ev, vec![GestureEvent { id: lp, state: GestureState::Changed }]);
        let ev = a.feed(&TouchSample::ended(1, 100.0, 0.0, 0.8));
        assert_eq!(ev, vec![GestureEvent { id: lp, state: GestureState::Ended }]);
    }

    #[test]
    fn pan_wins_when_the_finger_moves_before_the_press_time() {
        let mut a = GestureArbiter::new();
        let lp = a.add(LongPress::new());
        let pan = a.add(Pan::new());
        a.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        let ev = a.feed(&TouchSample::moved(1, 30.0, 0.0, 0.1));
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, lp), vec![GestureState::Failed]);
    }

    #[test]
    fn a_pan_that_began_keeps_the_touches_and_the_pinch_stays_failed() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        let pinch = a.add(Pinch::new());
        let ev = feed_all(&mut a, &[TouchSample::began(1, 100.0, 100.0, 0.0), TouchSample::moved(1, 130.0, 100.0, 0.02)]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, pinch), vec![GestureState::Failed]);
        a.feed(&TouchSample::began(2, 300.0, 100.0, 0.03));
        let ev = a.feed(&TouchSample::moved(2, 350.0, 100.0, 0.04));
        assert!(states_of(&ev, pinch).is_empty());
        assert_eq!(a.state(pan), GestureState::Changed);
    }

    #[test]
    fn two_that_come_due_together_do_not_both_begin() {
        let mut a = GestureArbiter::new();
        let first = a.add(LongPress::new());
        let second = a.add(LongPress::new());
        a.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        let ev = a.tick(0.6);
        assert_eq!(states_of(&ev, first), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, second), vec![GestureState::Failed], "it never began, so no Cancelled either");
    }

    #[test]
    fn simultaneous_recognizers_run_together() {
        let mut a = GestureArbiter::new();
        let pinch = a.add(Pinch::new());
        let rot = a.add(Rotation::new());
        a.allow_simultaneous(pinch, rot);
        feed_all(&mut a, &[TouchSample::began(1, 100.0, 0.0, 0.0), TouchSample::began(2, 200.0, 0.0, 0.0)]);
        a.feed(&TouchSample::moved(2, 220.0, 60.0, 0.05));
        assert!(a.state(pinch).is_active());
        assert!(a.state(rot).is_active());
        assert!(a.get::<Pinch>(pinch).unwrap().scale() > 1.3);
        assert!(a.get::<Rotation>(rot).unwrap().rotation() > 0.4);
    }

    #[test]
    fn without_permission_the_first_to_begin_excludes_the_other() {
        let mut a = GestureArbiter::new();
        let pinch = a.add(Pinch::new());
        let rot = a.add(Rotation::new());
        feed_all(&mut a, &[TouchSample::began(1, 100.0, 0.0, 0.0), TouchSample::began(2, 200.0, 0.0, 0.0)]);
        let ev = a.feed(&TouchSample::moved(2, 220.0, 60.0, 0.05));
        assert_eq!(states_of(&ev, pinch), vec![GestureState::Began]);
        assert_eq!(states_of(&ev, rot), vec![GestureState::Failed], "it never began");
    }

    #[test]
    fn disabled_recognizers_do_nothing_and_do_not_block() {
        let (mut a, single, double) = tap_pair();
        a.set_enabled(double, false);
        let ev = feed_all(&mut a, &tap(1, 0.0, 0.0, 0.0));
        assert_eq!(states_of(&ev, single), vec![GestureState::Recognized], "no wait for a disabled double tap");
        assert_eq!(a.state(double), GestureState::Possible);
        assert!(!a.is_enabled(double));
        assert_eq!(a.next_deadline(), None);
    }

    #[test]
    fn disabling_a_running_recognizer_cancels_it() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        feed_all(&mut a, &[TouchSample::began(1, 0.0, 0.0, 0.0), TouchSample::moved(1, 30.0, 0.0, 0.02)]);
        let ev = a.set_enabled(pan, false);
        assert_eq!(ev, vec![GestureEvent { id: pan, state: GestureState::Cancelled }]);
        let ev = a.feed(&TouchSample::moved(1, 60.0, 0.0, 0.04));
        assert!(ev.is_empty());
    }

    #[test]
    fn cancel_all_cancels_and_restarts_everything() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        let tap_id = a.add(Tap::new(1, 1));
        feed_all(&mut a, &[TouchSample::began(1, 0.0, 0.0, 0.0), TouchSample::moved(1, 30.0, 0.0, 0.02)]);
        let ev = a.cancel_all();
        assert!(states_of(&ev, pan).contains(&GestureState::Cancelled));
        assert_eq!(a.state(pan), GestureState::Possible);
        let ev = feed_all(&mut a, &tap(2, 5.0, 5.0, 1.0));
        assert_eq!(states_of(&ev, tap_id), vec![GestureState::Recognized]);
    }

    #[test]
    fn a_new_cycle_resets_recognizers_that_finished() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        feed_all(&mut a, &[TouchSample::began(1, 0.0, 0.0, 0.0), TouchSample::moved(1, 30.0, 0.0, 0.02), TouchSample::ended(1, 30.0, 0.0, 0.03)]);
        assert_eq!(a.state(pan), GestureState::Ended);
        let ev = feed_all(&mut a, &[TouchSample::began(2, 0.0, 0.0, 1.0), TouchSample::moved(2, 30.0, 0.0, 1.02)]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Began]);
        assert_eq!(a.get::<Pan>(pan).unwrap().translation(), (30.0, 0.0));
    }

    #[test]
    fn the_pointer_drives_the_arbiter_like_a_touch() {
        let mut a = GestureArbiter::new();
        let tap_id = a.add(Tap::new(1, 1));
        let pan = a.add(Pan::new());
        let mut p = PointerAdapter::new();
        a.feed(&p.press(10.0, 10.0, 0.0));
        let ev = a.feed(&p.release(10.0, 10.0, 0.05).unwrap());
        assert_eq!(states_of(&ev, tap_id), vec![GestureState::Recognized]);
        assert_eq!(states_of(&ev, pan), vec![GestureState::Failed]);
    }

    #[test]
    fn typed_access_checks_the_type() {
        let mut a = GestureArbiter::new();
        let pan = a.add(Pan::new());
        assert!(a.get::<Pan>(pan).is_some());
        assert!(a.get::<Tap>(pan).is_none());
        assert!(a.get::<Pan>(9).is_none());
        a.get_mut::<Pan>(pan).unwrap().slop = 50.0;
        assert_eq!(a.get::<Pan>(pan).unwrap().slop, 50.0);
        assert_eq!(a.len(), 1);
        assert!(!a.is_empty());
    }

    #[test]
    #[should_panic(expected = "bad recognizer ids")]
    fn a_recognizer_cannot_require_itself() {
        let mut a = GestureArbiter::new();
        let t = a.add(Tap::new(1, 1));
        a.require_failure(t, t);
    }
}
