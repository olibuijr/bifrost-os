//! Schedulers: where a unit of work runs and when. A [`Scheduler`] takes work and runs it now or after a delay on a
//! thread of its choice. Two schedulers come with it: [`ImmediateScheduler`], which runs work on the calling thread
//! before `schedule` returns, and every [`DispatchQueue`], which runs work on that queue. The main queue is the app
//! loop, so `DispatchQueue::main()` is the run-loop scheduler.
//!
//! Publishers in [`crate::observable`] are single-threaded and do not move values between threads, so there is no
//! `receive(on:)` or `subscribe(on:)` operator here. An app that needs a value on another scheduler sends it to that
//! scheduler's work by hand.

use std::time::Duration;

use crate::dispatch::DispatchQueue;

/// Runs units of work now or later.
pub trait Scheduler {
    /// Runs `work` as soon as the scheduler can.
    fn schedule<F: FnOnce() + Send + 'static>(&self, work: F);

    /// Runs `work` no earlier than `delay` and no later than `delay` plus `tolerance`.
    fn schedule_after<F: FnOnce() + Send + 'static>(&self, delay: Duration, tolerance: Duration, work: F);
}

/// Runs work on the calling thread before the call returns. It does not wait out a delay: the delayed form runs the
/// work at once, so an app that needs real timing uses a [`DispatchQueue`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImmediateScheduler;

impl Scheduler for ImmediateScheduler {
    fn schedule<F: FnOnce() + Send + 'static>(&self, work: F) {
        work();
    }

    fn schedule_after<F: FnOnce() + Send + 'static>(&self, _delay: Duration, _tolerance: Duration, work: F) {
        work();
    }
}

impl Scheduler for DispatchQueue {
    fn schedule<F: FnOnce() + Send + 'static>(&self, work: F) {
        self.async_(work);
    }

    fn schedule_after<F: FnOnce() + Send + 'static>(&self, delay: Duration, tolerance: Duration, work: F) {
        self.async_after_leeway(delay, tolerance, work);
    }
}
