//! Timer dispatch source (`DispatchSourceTimer`) as a pure state model, with its lifecycle (`DispatchSourceProtocol`):
//! activate, suspend, resume and cancel, plus the event and cancel handlers.
//!
//! Nothing here runs on a thread or a queue. The owner calls `advance(now)` from its own loop with the current
//! `DispatchTime`; that call decides whether the timer has expired and runs the event handler with the number of
//! expirations since the last delivery. Expirations that pass while the source is suspended are not lost: they
//! arrive as one event when the source is resumed, which is how coalescing works in the framework.
//!
//! Scope: only the timer source is modelled. File descriptor, signal, process and file system sources need kernel
//! objects, and they are not modelled here.
//!
//! Rules the model follows:
//! - A new source is inactive. `activate` starts it once; a second call is an error.
//! - `suspend` and `resume` nest. `resume` without a matching `suspend` is an error.
//! - `cancel` is final and idempotent. It drops the event handler and runs the cancel handler once.
//! - A repeating interval of zero or less, or `Never`, makes a one-shot timer.
//! - `leeway` is the latest the event may arrive after the deadline. `advance` fires at the deadline and never early.

use std::time::Duration;

use crate::dispatch_time::{DispatchTime, DispatchTimeInterval};

/// Errors from the source lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerSourceError {
    /// `activate` was called on a source that is already active.
    AlreadyActivated,
    /// `resume` was called with no suspension outstanding.
    ResumeWithoutSuspend,
    /// The source was cancelled, so it can no longer be suspended, resumed or activated.
    Cancelled,
    /// The suspension count would exceed `u32::MAX`.
    SuspendOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Schedule {
    /// The next deadline that has not been delivered yet.
    next: DispatchTime,
    /// The repeat period in nanoseconds; `None` for a one-shot timer.
    period: Option<u64>,
    leeway: Duration,
}

/// A timer dispatch source.
pub struct TimerSource {
    activated: bool,
    cancelled: bool,
    suspend_count: u32,
    schedule: Option<Schedule>,
    handler: Option<Box<dyn FnMut(u64) + Send>>,
    cancel_handler: Option<Box<dyn FnOnce() + Send>>,
}

impl Default for TimerSource {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerSource {
    /// A new, inactive timer with no schedule (`makeTimerSource`).
    pub fn new() -> Self {
        TimerSource {
            activated: false,
            cancelled: false,
            suspend_count: 0,
            schedule: None,
            handler: None,
            cancel_handler: None,
        }
    }

    /// Sets the deadline, the repeat interval and the leeway (`schedule(deadline:repeating:leeway:)`). A later call
    /// replaces the earlier schedule. `repeating` of `Never`, or of zero or less, makes a one-shot timer.
    pub fn schedule(&mut self, deadline: DispatchTime, repeating: DispatchTimeInterval, leeway: DispatchTimeInterval) {
        let period = match repeating.nanoseconds() {
            Some(ns) if ns > 0 => Some(u64::try_from(ns).unwrap_or(u64::MAX)),
            _ => None,
        };
        self.schedule = Some(Schedule { next: deadline, period, leeway: leeway.as_duration().unwrap_or(Duration::ZERO) });
    }

    /// The handler that runs on each event, with the number of expirations it covers (`setEventHandler`).
    pub fn set_event_handler<F: FnMut(u64) + Send + 'static>(&mut self, handler: F) {
        self.handler = Some(Box::new(handler));
    }

    /// The handler that runs once when the source is cancelled (`setCancelHandler`).
    pub fn set_cancel_handler<F: FnOnce() + Send + 'static>(&mut self, handler: F) {
        self.cancel_handler = Some(Box::new(handler));
    }

    /// Starts the source (`activate()`). Events can flow only after activation.
    pub fn activate(&mut self) -> Result<(), TimerSourceError> {
        if self.cancelled {
            return Err(TimerSourceError::Cancelled);
        }
        if self.activated {
            return Err(TimerSourceError::AlreadyActivated);
        }
        self.activated = true;
        Ok(())
    }

    /// Adds one suspension (`suspend()`). Nothing is delivered while the count is above zero.
    pub fn suspend(&mut self) -> Result<(), TimerSourceError> {
        if self.cancelled {
            return Err(TimerSourceError::Cancelled);
        }
        self.suspend_count = self.suspend_count.checked_add(1).ok_or(TimerSourceError::SuspendOverflow)?;
        Ok(())
    }

    /// Removes one suspension (`resume()`).
    pub fn resume(&mut self) -> Result<(), TimerSourceError> {
        if self.cancelled {
            return Err(TimerSourceError::Cancelled);
        }
        if self.suspend_count == 0 {
            return Err(TimerSourceError::ResumeWithoutSuspend);
        }
        self.suspend_count -= 1;
        Ok(())
    }

    /// Cancels the source (`cancel()`). The event handler is dropped, and the cancel handler runs once. Repeated calls
    /// do nothing.
    pub fn cancel(&mut self) {
        if self.cancelled {
            return;
        }
        self.cancelled = true;
        self.handler = None;
        if let Some(f) = self.cancel_handler.take() {
            f();
        }
    }

    /// `isCancelled`.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// The deadline the next event is due at, or `None` when there is no schedule or the one-shot has fired.
    pub fn next_deadline(&self) -> Option<DispatchTime> {
        self.schedule.map(|s| s.next).filter(|t| *t != DispatchTime::DISTANT_FUTURE)
    }

    /// The latest time the next event may be delivered: its deadline plus the leeway.
    pub fn latest_delivery(&self) -> Option<DispatchTime> {
        let next = self.next_deadline()?;
        Some(next.advanced_by(DispatchTimeInterval::Nanoseconds(saturating_nanos(self.leeway_of()))))
    }

    fn leeway_of(&self) -> Duration {
        self.schedule.map_or(Duration::ZERO, |s| s.leeway)
    }


    /// Checks the clock. If the deadline has passed (and the source is active and not suspended), the event handler
    /// runs with the number of expirations it covers, and the count is returned. Otherwise returns 0.
    pub fn advance(&mut self, now: DispatchTime) -> u64 {
        if self.cancelled || !self.activated || self.suspend_count > 0 {
            return 0;
        }
        let count = {
            let Some(s) = self.schedule.as_mut() else {
                return 0;
            };
            if s.next == DispatchTime::DISTANT_FUTURE || now < s.next {
                return 0;
            }
            match s.period {
                None => {
                    s.next = DispatchTime::DISTANT_FUTURE;
                    1
                }
                Some(period) => {
                    let late = now.uptime_nanos() - s.next.uptime_nanos();
                    let count = 1u64.saturating_add(late / period);
                    let step = count.saturating_mul(period);
                    s.next = DispatchTime::from_uptime_nanos(s.next.uptime_nanos().saturating_add(step));
                    count
                }
            }
        };
        if let Some(handler) = self.handler.as_mut() {
            handler(count);
        }
        count
    }
}

fn saturating_nanos(d: Duration) -> i64 {
    i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn at(ns: u64) -> DispatchTime {
        DispatchTime::from_uptime_nanos(ns)
    }

    #[test]
    fn a_one_shot_fires_once_at_its_deadline() {
        let seen = Arc::new(AtomicU64::new(0));
        let mut t = TimerSource::new();
        let s = seen.clone();
        t.set_event_handler(move |n| {
            s.fetch_add(n, Ordering::SeqCst);
        });
        t.schedule(at(1_000), DispatchTimeInterval::Never, DispatchTimeInterval::Nanoseconds(0));
        t.activate().unwrap();
        assert_eq!(t.advance(at(999)), 0);
        assert_eq!(t.advance(at(1_000)), 1);
        assert_eq!(t.advance(at(5_000)), 0);
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        assert_eq!(t.next_deadline(), None);
    }

    #[test]
    fn a_repeating_timer_coalesces_missed_periods_into_one_event() {
        let mut t = TimerSource::new();
        t.schedule(at(1_000), DispatchTimeInterval::Nanoseconds(100), DispatchTimeInterval::Nanoseconds(0));
        t.activate().unwrap();
        assert_eq!(t.advance(at(1_000)), 1);
        assert_eq!(t.next_deadline(), Some(at(1_100)));
        // Expirations at 1100, 1200 and 1300 are due by 1350: one event covering three.
        assert_eq!(t.advance(at(1_350)), 3);
        assert_eq!(t.next_deadline(), Some(at(1_400)));
        assert_eq!(t.advance(at(1_399)), 0);
    }

    #[test]
    fn nothing_fires_before_activation() {
        let mut t = TimerSource::new();
        t.schedule(at(10), DispatchTimeInterval::Never, DispatchTimeInterval::Never);
        assert_eq!(t.advance(at(100)), 0);
        assert_eq!(t.activate(), Ok(()));
        assert_eq!(t.activate(), Err(TimerSourceError::AlreadyActivated));
        assert_eq!(t.advance(at(100)), 1);
    }

    #[test]
    fn suspended_expirations_arrive_as_one_event_on_resume() {
        let mut t = TimerSource::new();
        t.schedule(at(100), DispatchTimeInterval::Nanoseconds(10), DispatchTimeInterval::Never);
        t.activate().unwrap();
        t.suspend().unwrap();
        t.suspend().unwrap();
        assert_eq!(t.advance(at(200)), 0);
        t.resume().unwrap();
        assert_eq!(t.advance(at(200)), 0, "still suspended once");
        t.resume().unwrap();
        // Expirations at 100, 110, ..., 200: eleven of them, delivered together.
        assert_eq!(t.advance(at(200)), 11);
        assert_eq!(t.resume(), Err(TimerSourceError::ResumeWithoutSuspend));
    }

    #[test]
    fn cancel_is_final_and_runs_the_cancel_handler_once() {
        let cancels = Arc::new(AtomicU64::new(0));
        let mut t = TimerSource::new();
        let c = cancels.clone();
        t.set_cancel_handler(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        t.schedule(at(5), DispatchTimeInterval::Nanoseconds(5), DispatchTimeInterval::Never);
        t.activate().unwrap();
        t.cancel();
        t.cancel();
        assert!(t.is_cancelled());
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert_eq!(t.advance(at(1_000)), 0);
        assert_eq!(t.suspend(), Err(TimerSourceError::Cancelled));
        assert_eq!(t.resume(), Err(TimerSourceError::Cancelled));
    }

    #[test]
    fn the_latest_delivery_adds_the_leeway_to_the_deadline() {
        let mut t = TimerSource::new();
        t.schedule(at(1_000), DispatchTimeInterval::Never, DispatchTimeInterval::Microseconds(2));
        assert_eq!(t.latest_delivery(), Some(at(3_000)));
        t.activate().unwrap();
        // Fires at the deadline, never before it.
        assert_eq!(t.advance(at(999)), 0);
        assert_eq!(t.advance(at(1_000)), 1);
    }

    #[test]
    fn a_zero_repeat_interval_is_a_one_shot() {
        let mut t = TimerSource::new();
        t.schedule(at(1_000), DispatchTimeInterval::Nanoseconds(0), DispatchTimeInterval::Never);
        t.activate().unwrap();
        assert_eq!(t.advance(at(1_000)), 1);
        assert_eq!(t.advance(at(9_000)), 0);
    }
}
