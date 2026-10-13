//! Touch gesture recognizers shared by the shell and apps.
//!
//! A recognizer is a small state machine fed with [`TouchSample`]s (touch id, phase, position, time).
//! A pointer is just one touch with a fixed id, see [`PointerAdapter`], so the same recognizers serve a
//! touchscreen and a mouse. Times are seconds, distances points, velocities points (or radians, or scale
//! units) per second.
//!
//! The recognizers are pure: they never read a clock. Callers pass the sample time and, for time based
//! decisions such as a long press or the gap between taps, call [`Recognizer::tick`] from their frame or timer
//! loop; [`Recognizer::deadline`] says when the next tick matters.
//!
//! State flow:
//! * discrete gestures (tap, swipe) go `Possible -> Recognized` or `Possible -> Failed`;
//! * continuous gestures (long press, pan, pinch, rotation, screen-edge pan) go
//!   `Possible -> Began -> Changed* -> Ended`, `-> Cancelled` when interrupted, or `Possible -> Failed`.
//!
//! A terminal state stays readable until the next touch cycle starts, then the recognizer resets itself.
//!
//! [`GestureArbiter`] runs a set of recognizers together: one recognizer can require another to fail first,
//! and recognizers recognize exclusively unless a pair is allowed to run simultaneously.

use std::any::Any;
use std::collections::VecDeque;

macro_rules! recognizer_plumbing {
    () => {
        fn core(&self) -> &Core {
            &self.core
        }
        fn core_mut(&mut self) -> &mut Core {
            &mut self.core
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

mod arbiter;
mod edge;
mod long_press;
mod pan;
mod pinch;
mod rotation;
mod swipe;
mod tap;

pub use arbiter::{GestureArbiter, GestureEvent};
pub use edge::{Edge, EdgeMask, ScreenEdgePan};
pub use long_press::LongPress;
pub use pan::Pan;
pub use pinch::Pinch;
pub use rotation::{Rotation, ROTATION_SLOP};
pub use swipe::{Direction, DirectionMask, Swipe};
pub use tap::Tap;

/// Smoothing window of the velocity estimate: a finger that rested this long has no velocity.
pub const PAUSE_SECS: f64 = 0.12;
/// The id the pointer adapter gives its single touch.
pub const POINTER_ID: u64 = u64::MAX;

/// What a touch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    /// The system took the touch away (focus change, palm rejection).
    Cancelled,
}

/// One input sample of one touch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchSample {
    pub id: u64,
    pub phase: TouchPhase,
    pub x: f32,
    pub y: f32,
    /// Seconds on any monotonic clock.
    pub t: f64,
}

impl TouchSample {
    pub fn new(id: u64, phase: TouchPhase, x: f32, y: f32, t: f64) -> Self {
        TouchSample { id, phase, x, y, t }
    }
    pub fn began(id: u64, x: f32, y: f32, t: f64) -> Self {
        Self::new(id, TouchPhase::Began, x, y, t)
    }
    pub fn moved(id: u64, x: f32, y: f32, t: f64) -> Self {
        Self::new(id, TouchPhase::Moved, x, y, t)
    }
    pub fn ended(id: u64, x: f32, y: f32, t: f64) -> Self {
        Self::new(id, TouchPhase::Ended, x, y, t)
    }
    pub fn cancelled(id: u64, x: f32, y: f32, t: f64) -> Self {
        Self::new(id, TouchPhase::Cancelled, x, y, t)
    }
}

/// Turns pointer button and motion events into samples of one touch.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerAdapter {
    down: bool,
}

impl PointerAdapter {
    pub fn new() -> Self {
        Self::default()
    }
    /// The primary button went down.
    pub fn press(&mut self, x: f32, y: f32, t: f64) -> TouchSample {
        self.down = true;
        TouchSample::began(POINTER_ID, x, y, t)
    }
    /// The pointer moved; a sample only while the button is held.
    pub fn motion(&mut self, x: f32, y: f32, t: f64) -> Option<TouchSample> {
        self.down.then(|| TouchSample::moved(POINTER_ID, x, y, t))
    }
    /// The button went up; a sample only if it was down.
    pub fn release(&mut self, x: f32, y: f32, t: f64) -> Option<TouchSample> {
        if !self.down {
            return None;
        }
        self.down = false;
        Some(TouchSample::ended(POINTER_ID, x, y, t))
    }
    /// The pointer left or the grab was lost.
    pub fn cancel(&mut self, x: f32, y: f32, t: f64) -> Option<TouchSample> {
        if !self.down {
            return None;
        }
        self.down = false;
        Some(TouchSample::cancelled(POINTER_ID, x, y, t))
    }
    pub fn is_down(&self) -> bool {
        self.down
    }
}

/// Where a recognizer is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureState {
    /// Waiting for input that matches.
    #[default]
    Possible,
    /// A continuous gesture started.
    Began,
    /// A continuous gesture moved on.
    Changed,
    /// A continuous gesture finished normally.
    Ended,
    /// A continuous gesture was interrupted.
    Cancelled,
    /// The input does not match this gesture.
    Failed,
    /// A discrete gesture happened.
    Recognized,
}

impl GestureState {
    /// No further change until the recognizer resets.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Ended | Self::Cancelled | Self::Failed | Self::Recognized)
    }
    /// A continuous gesture is in progress.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Began | Self::Changed)
    }
    /// The gesture was (or is being) recognized.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Began | Self::Changed | Self::Ended | Self::Recognized)
    }
    /// The gesture did not happen.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled)
    }
}

/// State shared by every recognizer: the published state plus the transitions held back while the recognizer
/// must wait for another one to fail.
#[derive(Debug, Default)]
pub struct Core {
    state: GestureState,
    /// Transitions proposed while waiting; the observer has seen none of them yet.
    queue: VecDeque<GestureState>,
    gated: bool,
    logging: bool,
    log: Vec<GestureState>,
    began: bool,
}

impl Core {
    pub fn new() -> Self {
        Self::default()
    }
    /// The state the observer sees.
    pub fn state(&self) -> GestureState {
        self.state
    }
    /// The state the recognizer logic has reached, including held back transitions.
    pub fn current(&self) -> GestureState {
        self.queue.back().copied().unwrap_or(self.state)
    }
    /// Transitions are being held back.
    pub fn busy(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Proposes a transition. Terminal states are final until [`Core::reset`].
    pub fn go(&mut self, next: GestureState) {
        if self.current().is_terminal() {
            return;
        }
        let unpublished = self.state == GestureState::Possible;
        if unpublished && (self.gated || !self.queue.is_empty()) {
            match next {
                GestureState::Failed | GestureState::Cancelled => {
                    self.queue.clear();
                    self.publish(GestureState::Failed);
                }
                GestureState::Changed => {
                    if self.queue.back() != Some(&GestureState::Changed) {
                        self.queue.push_back(next);
                    }
                }
                _ => self.queue.push_back(next),
            }
            return;
        }
        self.publish(next);
    }

    fn publish(&mut self, next: GestureState) {
        self.state = next;
        if matches!(next, GestureState::Began | GestureState::Recognized) {
            self.began = true;
        }
        if self.logging {
            self.log.push(next);
        }
    }

    pub(super) fn set_gated(&mut self, gated: bool) {
        self.gated = gated;
    }
    pub(super) fn set_logging(&mut self, on: bool) {
        self.logging = on;
    }
    pub(super) fn take_log(&mut self) -> Vec<GestureState> {
        std::mem::take(&mut self.log)
    }
    /// True once after a Began or Recognized was published.
    pub(super) fn take_began(&mut self) -> bool {
        std::mem::take(&mut self.began)
    }
    /// Publishes everything that was held back.
    pub(super) fn release(&mut self) {
        while let Some(next) = self.queue.pop_front() {
            self.publish(next);
        }
    }
    /// Fails a recognizer that has shown nothing yet, cancels one in progress.
    pub(super) fn force_fail(&mut self) {
        self.began = false;
        match self.state {
            GestureState::Possible => {
                self.queue.clear();
                self.publish(GestureState::Failed);
            }
            s if s.is_active() => {
                self.queue.clear();
                self.publish(GestureState::Cancelled);
            }
            _ => {}
        }
    }
    pub fn reset(&mut self) {
        let (logging, gated) = (self.logging, self.gated);
        let log = std::mem::take(&mut self.log);
        *self = Core { logging, gated, log, ..Core::default() };
    }
}

/// A gesture recognizer. See the module docs.
pub trait Recognizer: Any {
    fn core(&self) -> &Core;
    fn core_mut(&mut self) -> &mut Core;
    /// Consumes one touch sample.
    fn feed(&mut self, s: &TouchSample);
    /// Lets time pass without input.
    fn tick(&mut self, t: f64);
    /// The time at which [`Recognizer::tick`] will change something, if any.
    fn deadline(&self) -> Option<f64> {
        None
    }
    /// Back to `Possible` with all tracking dropped.
    fn reset(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn state(&self) -> GestureState {
        self.core().state()
    }
}

/// True when a sample starts a new touch cycle on a recognizer that finished the last one.
fn needs_reset(core: &Core, touches: &TouchSet, s: &TouchSample) -> bool {
    s.phase == TouchPhase::Began && touches.is_empty() && core.current().is_terminal() && !core.busy()
}

pub(crate) fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Smoothed rate of change of one value; the math of the shell's system gestures, per second.
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    x: f32,
    t: f64,
    v: f32,
}

impl Smoother {
    pub fn new(x: f32, t: f64) -> Self {
        Smoother { x, t, v: 0.0 }
    }
    /// Blends in a sample; after a pause the old rate no longer counts.
    pub fn push(&mut self, x: f32, t: f64) {
        let dt = t - self.t;
        if dt > 0.0 {
            let now = (x - self.x) / dt as f32;
            self.v = if dt > PAUSE_SECS { now } else { 0.6 * now + 0.4 * self.v };
            self.x = x;
            self.t = t;
        }
    }
    /// The rate at `t`: zero if the value rested first.
    pub fn at(&self, t: f64) -> f32 {
        if t - self.t > PAUSE_SECS {
            0.0
        } else {
            self.v
        }
    }
}

/// Smoothed 2D velocity.
#[derive(Clone, Copy, Debug)]
pub struct Velocity2 {
    x: Smoother,
    y: Smoother,
}

impl Velocity2 {
    pub fn new(x: f32, y: f32, t: f64) -> Self {
        Velocity2 { x: Smoother::new(x, t), y: Smoother::new(y, t) }
    }
    pub fn push(&mut self, x: f32, y: f32, t: f64) {
        self.x.push(x, t);
        self.y.push(y, t);
    }
    pub fn at(&self, t: f64) -> (f32, f32) {
        (self.x.at(t), self.y.at(t))
    }
}

/// One finger (or the pointer) being tracked.
#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub id: u64,
    pub start: (f32, f32),
    pub start_t: f64,
    pub pos: (f32, f32),
    pub t: f64,
    vel: Velocity2,
}

impl Touch {
    /// Distance from where the touch began.
    pub fn travel(&self) -> f32 {
        dist(self.pos, self.start)
    }
    pub fn velocity_at(&self, t: f64) -> (f32, f32) {
        self.vel.at(t)
    }
}

/// The touches currently down, in arrival order.
#[derive(Clone, Debug, Default)]
pub struct TouchSet {
    touches: Vec<Touch>,
}

impl TouchSet {
    pub fn len(&self) -> usize {
        self.touches.len()
    }
    pub fn is_empty(&self) -> bool {
        self.touches.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Touch> {
        self.touches.iter()
    }
    pub fn get(&self, id: u64) -> Option<&Touch> {
        self.touches.iter().find(|t| t.id == id)
    }
    pub fn clear(&mut self) {
        self.touches.clear();
    }
    /// Records a sample. False if it belongs to a touch that was never seen beginning.
    pub fn apply(&mut self, s: &TouchSample) -> bool {
        if let Some(t) = self.touches.iter_mut().find(|t| t.id == s.id) {
            // A release at the last position carries no motion and must not dilute the speed of the flick.
            let rest = s.phase == TouchPhase::Ended && t.pos == (s.x, s.y);
            t.pos = (s.x, s.y);
            t.t = s.t;
            if !rest {
                t.vel.push(s.x, s.y, s.t);
            }
            return true;
        }
        if s.phase == TouchPhase::Began {
            self.touches.push(Touch {
                id: s.id,
                start: (s.x, s.y),
                start_t: s.t,
                pos: (s.x, s.y),
                t: s.t,
                vel: Velocity2::new(s.x, s.y, s.t),
            });
            return true;
        }
        false
    }
    /// Drops the touch of an ended or cancelled sample.
    pub fn finish(&mut self, s: &TouchSample) {
        if matches!(s.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.touches.retain(|t| t.id != s.id);
        }
    }
    /// Mean position of the touches; the origin when none is down.
    pub fn centroid(&self) -> (f32, f32) {
        if self.touches.is_empty() {
            return (0.0, 0.0);
        }
        let n = self.touches.len() as f32;
        let (sx, sy) = self.touches.iter().fold((0.0, 0.0), |a, t| (a.0 + t.pos.0, a.1 + t.pos.1));
        (sx / n, sy / n)
    }
    /// Mean velocity of the touches at `t`.
    pub fn velocity_at(&self, t: f64) -> (f32, f32) {
        if self.touches.is_empty() {
            return (0.0, 0.0);
        }
        let n = self.touches.len() as f32;
        let (sx, sy) = self.touches.iter().fold((0.0, 0.0), |a, k| {
            let v = k.velocity_at(t);
            (a.0 + v.0, a.1 + v.1)
        });
        (sx / n, sy / n)
    }
}

/// The two touches a pinch or rotation follows: the first two to arrive.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TwoTouch {
    ids: [Option<u64>; 2],
}

impl TwoTouch {
    pub fn add(&mut self, id: u64) {
        if self.has(id) {
            return;
        }
        if let Some(slot) = self.ids.iter_mut().find(|s| s.is_none()) {
            *slot = Some(id);
        }
    }
    pub fn has(&self, id: u64) -> bool {
        self.ids.contains(&Some(id))
    }
    pub fn positions(&self, set: &TouchSet) -> Option<((f32, f32), (f32, f32))> {
        let a = set.get(self.ids[0]?)?;
        let b = set.get(self.ids[1]?)?;
        Some((a.pos, b.pos))
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// Feeds `samples` to a recognizer.
    pub fn run<R: Recognizer>(r: &mut R, samples: &[TouchSample]) {
        for s in samples {
            r.feed(s);
        }
    }

    /// Samples moving touch `id` from `a` to `b` in `steps` steps between `t0` and `t1`.
    pub fn drag(id: u64, a: (f32, f32), b: (f32, f32), t0: f64, t1: f64, steps: usize) -> Vec<TouchSample> {
        (1..=steps)
            .map(|i| {
                let k = i as f32 / steps as f32;
                let t = t0 + (t1 - t0) * i as f64 / steps as f64;
                TouchSample::moved(id, a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, t)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_classes() {
        assert!(GestureState::Failed.is_terminal() && GestureState::Failed.is_failure());
        assert!(GestureState::Recognized.is_terminal() && GestureState::Recognized.is_success());
        assert!(GestureState::Began.is_active() && !GestureState::Began.is_terminal());
        assert!(!GestureState::Possible.is_success() && !GestureState::Possible.is_failure());
        assert!(GestureState::Cancelled.is_failure());
        assert_eq!(GestureState::default(), GestureState::Possible);
    }

    #[test]
    fn core_publishes_immediately_when_not_gated() {
        let mut c = Core::new();
        c.go(GestureState::Began);
        assert_eq!(c.state(), GestureState::Began);
        c.go(GestureState::Changed);
        c.go(GestureState::Ended);
        assert_eq!(c.state(), GestureState::Ended);
        c.go(GestureState::Changed);
        assert_eq!(c.state(), GestureState::Ended, "a terminal state is final");
        c.reset();
        assert_eq!(c.state(), GestureState::Possible);
    }

    #[test]
    fn core_holds_transitions_while_gated_and_coalesces_changes() {
        let mut c = Core::new();
        c.set_logging(true);
        c.set_gated(true);
        c.go(GestureState::Began);
        c.go(GestureState::Changed);
        c.go(GestureState::Changed);
        c.go(GestureState::Ended);
        assert_eq!(c.state(), GestureState::Possible, "nothing visible yet");
        assert_eq!(c.current(), GestureState::Ended);
        assert!(c.busy());
        c.set_gated(false);
        c.release();
        assert_eq!(c.state(), GestureState::Ended);
        assert_eq!(c.take_log(), vec![GestureState::Began, GestureState::Changed, GestureState::Ended]);
    }

    #[test]
    fn core_failure_is_never_held_and_drops_the_queue() {
        let mut c = Core::new();
        c.set_gated(true);
        c.go(GestureState::Recognized);
        c.go(GestureState::Failed);
        assert_eq!(c.state(), GestureState::Possible, "a held result is final; only force_fail overrides it");
        assert_eq!(c.current(), GestureState::Recognized);
        c.force_fail();
        assert_eq!(c.state(), GestureState::Failed);
        assert!(!c.busy());
        let mut d = Core::new();
        d.set_gated(true);
        d.go(GestureState::Began);
        d.go(GestureState::Cancelled);
        assert_eq!(d.state(), GestureState::Failed, "a held gesture that is cancelled never began");
        assert!(!d.busy());
    }

    #[test]
    fn reset_keeps_the_gate_and_the_log_settings() {
        let mut c = Core::new();
        c.set_logging(true);
        c.set_gated(true);
        c.go(GestureState::Began);
        c.reset();
        c.go(GestureState::Recognized);
        assert_eq!(c.state(), GestureState::Possible, "still gated after a reset");
        assert!(c.busy());
    }

    #[test]
    fn force_fail_fails_unpublished_and_cancels_active() {
        let mut c = Core::new();
        c.force_fail();
        assert_eq!(c.state(), GestureState::Failed);
        let mut d = Core::new();
        d.go(GestureState::Began);
        d.force_fail();
        assert_eq!(d.state(), GestureState::Cancelled);
        let mut e = Core::new();
        e.go(GestureState::Recognized);
        e.force_fail();
        assert_eq!(e.state(), GestureState::Recognized, "finished gestures stay");
    }

    #[test]
    fn smoother_follows_a_steady_rate_and_forgets_after_a_pause() {
        let mut s = Smoother::new(0.0, 0.0);
        for i in 1..=10 {
            s.push(i as f32 * 10.0, i as f64 * 0.01);
        }
        assert!((s.at(0.1) - 1000.0).abs() < 1.0, "10 units per 10 ms is 1000 per second");
        assert_eq!(s.at(0.1 + PAUSE_SECS + 0.01), 0.0);
        s.push(100.0, 0.1 + PAUSE_SECS + 0.01);
        assert_eq!(s.at(0.1 + PAUSE_SECS + 0.01), 0.0, "no motion after the pause");
    }

    #[test]
    fn touch_set_tracks_arrival_order_centroid_and_velocity() {
        let mut set = TouchSet::default();
        assert!(set.apply(&TouchSample::began(1, 0.0, 0.0, 0.0)));
        assert!(set.apply(&TouchSample::began(2, 100.0, 40.0, 0.0)));
        assert!(!set.apply(&TouchSample::moved(9, 5.0, 5.0, 0.0)), "unknown touch");
        assert_eq!(set.len(), 2);
        assert_eq!(set.centroid(), (50.0, 20.0));
        assert!(set.apply(&TouchSample::moved(1, 10.0, 0.0, 0.01)));
        assert_eq!(set.get(1).unwrap().travel(), 10.0);
        assert!(set.velocity_at(0.01).0 > 0.0);
        set.finish(&TouchSample::ended(1, 10.0, 0.0, 0.02));
        assert_eq!(set.len(), 1);
        assert_eq!(set.iter().next().unwrap().id, 2);
        set.finish(&TouchSample::moved(2, 0.0, 0.0, 0.02));
        assert_eq!(set.len(), 1, "a move does not remove");
    }

    #[test]
    fn pointer_adapter_only_reports_while_pressed() {
        let mut p = PointerAdapter::new();
        assert!(p.motion(1.0, 1.0, 0.0).is_none());
        assert!(p.release(1.0, 1.0, 0.0).is_none());
        let down = p.press(5.0, 6.0, 0.1);
        assert_eq!(down.phase, TouchPhase::Began);
        assert_eq!(down.id, POINTER_ID);
        assert!(p.is_down());
        assert_eq!(p.motion(7.0, 8.0, 0.2).unwrap().phase, TouchPhase::Moved);
        assert_eq!(p.release(7.0, 8.0, 0.3).unwrap().phase, TouchPhase::Ended);
        assert!(!p.is_down());
        p.press(0.0, 0.0, 1.0);
        assert_eq!(p.cancel(0.0, 0.0, 1.1).unwrap().phase, TouchPhase::Cancelled);
    }

    #[test]
    fn two_touch_keeps_the_first_two() {
        let mut set = TouchSet::default();
        let mut pair = TwoTouch::default();
        for (id, x) in [(1u64, 0.0f32), (2, 10.0), (3, 20.0)] {
            set.apply(&TouchSample::began(id, x, 0.0, 0.0));
            pair.add(id);
        }
        assert!(pair.has(1) && pair.has(2) && !pair.has(3));
        assert_eq!(pair.positions(&set), Some(((0.0, 0.0), (10.0, 0.0))));
        set.finish(&TouchSample::ended(2, 10.0, 0.0, 0.1));
        assert_eq!(pair.positions(&set), None);
    }
}
