//! A main-thread run loop timer queue: the non-`Send` scheduler for publishers.
//!
//! [`crate::dispatch::DispatchQueue`] runs `Send` closures on worker threads, so the `Rc`-based publishers in
//! [`crate::observable`] cannot be scheduled on it. [`RunLoop`] is the other half: work that stays on the thread
//! that owns the publishers, due at a time the app supplies. Like [`crate::timer_source::TimerSource`], it has no
//! thread of its own. The app's tick calls [`RunLoop::advance`] with its clock, and that call runs every timer that
//! has come due, earliest first, then in the order they were scheduled.
//!
//! Rules:
//! - The clock only moves forward. `advance` with an earlier time keeps the latest time seen.
//! - A timer scheduled during `advance` waits for the next `advance`, so work that reschedules itself with zero
//!   delay cannot loop inside one tick.
//! - [`RunLoop::current`] is the queue of the calling thread. The app loop thread is the one whose tick drains it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

type Work = Box<dyn FnOnce()>;

struct Timer {
    due: Duration,
    seq: u64,
    work: Work,
}

struct Inner {
    now: Cell<Duration>,
    next_seq: Cell<u64>,
    timers: RefCell<Vec<Timer>>,
}

/// A handle to one timer queue. Clones share the queue. The handle is `!Send`, so it stays on its thread.
#[derive(Clone)]
pub struct RunLoop {
    inner: Rc<Inner>,
}

thread_local! {
    static CURRENT: RunLoop = RunLoop::new();
}

impl Default for RunLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl RunLoop {
    /// A new, empty queue whose clock reads zero.
    pub fn new() -> Self {
        RunLoop {
            inner: Rc::new(Inner {
                now: Cell::new(Duration::ZERO),
                next_seq: Cell::new(0),
                timers: RefCell::new(Vec::new()),
            }),
        }
    }

    /// The queue of the calling thread. The app's tick on its loop thread advances this one.
    pub fn current() -> Self {
        CURRENT.with(|c| c.clone())
    }

    /// The clock as of the last `advance`.
    pub fn now(&self) -> Duration {
        self.inner.now.get()
    }

    /// Timers that have not run yet.
    pub fn pending(&self) -> usize {
        self.inner.timers.borrow().len()
    }

    /// Runs `work` once the clock reaches `now() + delay`. The work runs on the next `advance` at or after that time.
    pub fn schedule_after(&self, delay: Duration, work: impl FnOnce() + 'static) {
        let seq = self.inner.next_seq.get();
        self.inner.next_seq.set(seq + 1);
        let due = self.now() + delay;
        self.inner.timers.borrow_mut().push(Timer { due, seq, work: Box::new(work) });
    }

    /// Moves the clock to `now` (never backwards) and runs the timers that are due, earliest first, ties in
    /// scheduling order. Timers scheduled by the work run in a later `advance`. Returns how many ran.
    pub fn advance(&self, now: Duration) -> usize {
        if now > self.now() {
            self.inner.now.set(now);
        }
        let clock = self.now();
        let limit = self.inner.next_seq.get();
        let mut ran = 0;
        loop {
            let next = {
                let mut timers = self.inner.timers.borrow_mut();
                let index = timers
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.due <= clock && t.seq < limit)
                    .min_by_key(|(_, t)| (t.due, t.seq))
                    .map(|(i, _)| i);
                index.map(|i| timers.remove(i))
            };
            let Some(timer) = next else { return ran };
            // The borrow above is released first, so the work may schedule or be cancelled freely.
            (timer.work)();
            ran += 1;
        }
    }
}

/// The main-thread counterpart of [`crate::schedulers::Scheduler`]: work that is not `Send` (it may hold `Rc`
/// publishers), run by the owner's tick. [`RunLoop`] is the one implementation.
pub trait LocalScheduler {
    /// Runs `work` no earlier than `delay` after the scheduler's current time.
    fn schedule_local_after(&self, delay: Duration, work: impl FnOnce() + 'static);
}

impl LocalScheduler for RunLoop {
    fn schedule_local_after(&self, delay: Duration, work: impl FnOnce() + 'static) {
        self.schedule_after(delay, work);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_scheduler_runs_work_on_the_run_loop_clock() {
        fn later<S: LocalScheduler>(s: &S, log: Rc<RefCell<Vec<&'static str>>>) {
            s.schedule_local_after(Duration::from_millis(5), move || log.borrow_mut().push("local"));
        }
        let rl = RunLoop::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        later(&rl, log.clone());
        assert_eq!(rl.advance(Duration::from_millis(4)), 0);
        assert_eq!(rl.advance(Duration::from_millis(5)), 1);
        assert_eq!(*log.borrow(), vec!["local"]);
    }

    fn log() -> (Rc<RefCell<Vec<&'static str>>>, Rc<RefCell<Vec<&'static str>>>) {
        let l = Rc::new(RefCell::new(Vec::new()));
        (l.clone(), l)
    }

    #[test]
    fn nothing_runs_before_it_is_due() {
        let rl = RunLoop::new();
        let (out, w) = log();
        rl.schedule_after(Duration::from_millis(100), move || w.borrow_mut().push("a"));
        assert_eq!(rl.advance(Duration::from_millis(99)), 0);
        assert!(out.borrow().is_empty());
        assert_eq!(rl.pending(), 1);
    }

    #[test]
    fn due_timers_run_earliest_first_then_in_scheduling_order() {
        let rl = RunLoop::new();
        let (out, w1) = log();
        let w2 = w1.clone();
        let w3 = w1.clone();
        rl.schedule_after(Duration::from_millis(30), move || w1.borrow_mut().push("late"));
        rl.schedule_after(Duration::from_millis(10), move || w2.borrow_mut().push("early-1"));
        rl.schedule_after(Duration::from_millis(10), move || w3.borrow_mut().push("early-2"));
        assert_eq!(rl.advance(Duration::from_millis(50)), 3);
        assert_eq!(*out.borrow(), vec!["early-1", "early-2", "late"]);
        assert_eq!(rl.pending(), 0);
    }

    #[test]
    fn delays_count_from_the_last_clock_reading() {
        let rl = RunLoop::new();
        rl.advance(Duration::from_millis(200));
        let (out, w) = log();
        rl.schedule_after(Duration::from_millis(50), move || w.borrow_mut().push("x"));
        assert_eq!(rl.advance(Duration::from_millis(249)), 0);
        assert_eq!(rl.advance(Duration::from_millis(250)), 1);
        assert_eq!(*out.borrow(), vec!["x"]);
    }

    #[test]
    fn the_clock_never_moves_backwards() {
        let rl = RunLoop::new();
        rl.advance(Duration::from_millis(500));
        rl.advance(Duration::from_millis(100));
        assert_eq!(rl.now(), Duration::from_millis(500));
    }

    #[test]
    fn work_scheduled_during_advance_waits_for_the_next_advance() {
        let rl = RunLoop::new();
        let (out, w) = log();
        let inner = rl.clone();
        rl.schedule_after(Duration::ZERO, move || {
            w.borrow_mut().push("first");
            let w2 = w.clone();
            inner.schedule_after(Duration::ZERO, move || w2.borrow_mut().push("second"));
        });
        assert_eq!(rl.advance(Duration::ZERO), 1);
        assert_eq!(*out.borrow(), vec!["first"]);
        assert_eq!(rl.advance(Duration::ZERO), 1);
        assert_eq!(*out.borrow(), vec!["first", "second"]);
    }

    #[test]
    fn current_is_one_queue_per_thread() {
        let a = RunLoop::current();
        let b = RunLoop::current();
        a.schedule_after(Duration::ZERO, || {});
        assert_eq!(b.pending(), 1);
        b.advance(Duration::ZERO);
        assert_eq!(a.pending(), 0);
    }
}
