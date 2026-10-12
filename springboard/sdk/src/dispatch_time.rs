//! Dispatch time values: `DispatchTime`, `DispatchWallTime`, `DispatchTimeInterval` and `DispatchTimeoutResult`.
//!
//! `DispatchTime` is a point on a monotonic clock with nanosecond precision. std has no boot-relative clock, so the
//! count starts when this module first reads the clock. `DispatchWallTime` is a point on the wall clock with
//! microsecond precision, counted from the Unix epoch. `DispatchTimeInterval` is a signed span in seconds,
//! milliseconds, microseconds or nanoseconds, or `Never`.
//!
//! All arithmetic saturates: a time never wraps, and `DISTANT_FUTURE` absorbs every later offset. Queues in
//! `dispatch.rs` take a `Duration`, so `duration_from` turns a deadline into the delay to pass them.

use std::ops::{Add, Sub};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A span of time: a count in one unit, or `Never` (no interval).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DispatchTimeInterval {
    Seconds(i64),
    Milliseconds(i64),
    Microseconds(i64),
    Nanoseconds(i64),
    Never,
}

impl DispatchTimeInterval {
    /// The span in nanoseconds, signed. `None` for `Never`. An `i128` holds any `i64` count scaled to nanoseconds.
    pub fn nanoseconds(self) -> Option<i128> {
        let (n, scale): (i64, i128) = match self {
            Self::Seconds(n) => (n, 1_000_000_000),
            Self::Milliseconds(n) => (n, 1_000_000),
            Self::Microseconds(n) => (n, 1_000),
            Self::Nanoseconds(n) => (n, 1),
            Self::Never => return None,
        };
        Some(i128::from(n) * scale)
    }

    /// The span as a `Duration`. A negative span becomes zero; `Never` gives `None`.
    pub fn as_duration(self) -> Option<Duration> {
        let ns = self.nanoseconds()?;
        Some(Duration::from_nanos(u64::try_from(ns.max(0)).unwrap_or(u64::MAX)))
    }

    /// The same span with the opposite sign. `Never` stays `Never`.
    fn negated(self) -> Self {
        match self {
            Self::Seconds(n) => Self::Seconds(n.saturating_neg()),
            Self::Milliseconds(n) => Self::Milliseconds(n.saturating_neg()),
            Self::Microseconds(n) => Self::Microseconds(n.saturating_neg()),
            Self::Nanoseconds(n) => Self::Nanoseconds(n.saturating_neg()),
            Self::Never => Self::Never,
        }
    }
}

/// A point on the monotonic clock, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DispatchTime(u64);

/// The clock's zero point: the first moment this module read the clock.
fn clock_origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

impl DispatchTime {
    /// A time after every other time (`DispatchTime.distantFuture`).
    pub const DISTANT_FUTURE: DispatchTime = DispatchTime(u64::MAX);

    /// The current time (`DispatchTime.now()`).
    pub fn now() -> Self {
        let ns = clock_origin().elapsed().as_nanos();
        DispatchTime(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    /// A time from a nanosecond count on the monotonic clock (`init(uptimeNanoseconds:)`).
    pub const fn from_uptime_nanos(nanos: u64) -> Self {
        DispatchTime(nanos)
    }

    /// The nanosecond count (`uptimeNanoseconds`).
    pub const fn uptime_nanos(self) -> u64 {
        self.0
    }

    /// The time moved by `interval` (`advanced(by:)`). `Never` gives `DISTANT_FUTURE`, and `DISTANT_FUTURE` stays
    /// where it is. The result is clamped to the range of the clock.
    pub fn advanced_by(self, interval: DispatchTimeInterval) -> Self {
        let Some(delta) = interval.nanoseconds() else {
            return Self::DISTANT_FUTURE;
        };
        if self == Self::DISTANT_FUTURE {
            return self;
        }
        let t = (i128::from(self.0) + delta).clamp(0, i128::from(u64::MAX));
        DispatchTime(t as u64)
    }

    /// The signed span from `self` to `other` (`distance(to:)`). `Never` when `other` is `DISTANT_FUTURE`.
    pub fn distance_to(self, other: DispatchTime) -> DispatchTimeInterval {
        if other == Self::DISTANT_FUTURE {
            return DispatchTimeInterval::Never;
        }
        let d = i128::from(other.0) - i128::from(self.0);
        let ns = i64::try_from(d).unwrap_or(if d < 0 { i64::MIN } else { i64::MAX });
        DispatchTimeInterval::Nanoseconds(ns)
    }

    /// How long from `now` until this time, for `DispatchQueue::async_after`. Zero when the time has passed.
    pub fn duration_from(self, now: DispatchTime) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(now.0))
    }
}

impl Add<DispatchTimeInterval> for DispatchTime {
    type Output = DispatchTime;
    fn add(self, rhs: DispatchTimeInterval) -> DispatchTime {
        self.advanced_by(rhs)
    }
}

impl Sub<DispatchTimeInterval> for DispatchTime {
    type Output = DispatchTime;
    fn sub(self, rhs: DispatchTimeInterval) -> DispatchTime {
        self.advanced_by(rhs.negated())
    }
}

/// A point on the wall clock, in microseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DispatchWallTime(i64);

impl DispatchWallTime {
    /// A time after every other wall time (`DispatchWallTime.distantFuture`).
    pub const DISTANT_FUTURE: DispatchWallTime = DispatchWallTime(i64::MAX);

    /// The current wall time (`DispatchWallTime.now()`). A clock set before 1970 reads as the epoch.
    pub fn now() -> Self {
        let micros = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
        DispatchWallTime(i64::try_from(micros).unwrap_or(i64::MAX))
    }

    /// From seconds and nanoseconds since the epoch (`init(timespec:)`). Nanoseconds below a microsecond are dropped.
    pub fn from_timespec(seconds: i64, nanoseconds: i64) -> Self {
        let micros = i128::from(seconds) * 1_000_000 + i128::from(nanoseconds) / 1_000;
        DispatchWallTime(i64::try_from(micros).unwrap_or(if micros < 0 { i64::MIN } else { i64::MAX }))
    }

    /// The count in microseconds since the epoch (`rawValue`).
    pub const fn unix_micros(self) -> i64 {
        self.0
    }

    /// The time moved by `interval`, truncated to microseconds. `Never` and `DISTANT_FUTURE` give `DISTANT_FUTURE`.
    pub fn advanced_by(self, interval: DispatchTimeInterval) -> Self {
        let Some(ns) = interval.nanoseconds() else {
            return Self::DISTANT_FUTURE;
        };
        if self == Self::DISTANT_FUTURE {
            return self;
        }
        let micros = i128::from(self.0) + ns / 1_000;
        DispatchWallTime(micros.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
    }
}

impl Add<DispatchTimeInterval> for DispatchWallTime {
    type Output = DispatchWallTime;
    fn add(self, rhs: DispatchTimeInterval) -> DispatchWallTime {
        self.advanced_by(rhs)
    }
}

/// The result of a wait that had a deadline (`DispatchTimeoutResult`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchTimeoutResult {
    /// The operation finished before the deadline.
    Success,
    /// The deadline passed first.
    TimedOut,
}

impl DispatchTimeoutResult {
    /// `Success` when the work finished in time, `TimedOut` otherwise.
    pub fn from_finished(finished: bool) -> Self {
        if finished { Self::Success } else { Self::TimedOut }
    }

    pub fn is_success(self) -> bool {
        self == Self::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_scale_to_nanoseconds_and_never_has_no_span() {
        assert_eq!(DispatchTimeInterval::Seconds(2).nanoseconds(), Some(2_000_000_000));
        assert_eq!(DispatchTimeInterval::Microseconds(1_500).nanoseconds(), Some(1_500_000));
        assert_eq!(DispatchTimeInterval::Nanoseconds(-7).nanoseconds(), Some(-7));
        assert_eq!(DispatchTimeInterval::Never.nanoseconds(), None);
        assert_eq!(DispatchTimeInterval::Milliseconds(-5).as_duration(), Some(Duration::ZERO));
        assert_eq!(DispatchTimeInterval::Never.as_duration(), None);
    }

    #[test]
    fn adding_and_subtracting_an_interval_moves_the_time() {
        let t = DispatchTime::from_uptime_nanos(1_000);
        assert_eq!((t + DispatchTimeInterval::Milliseconds(1)).uptime_nanos(), 1_001_000);
        assert_eq!((t - DispatchTimeInterval::Nanoseconds(400)).uptime_nanos(), 600);
        // A backward move past zero clamps at zero instead of wrapping.
        assert_eq!(DispatchTime::from_uptime_nanos(500).advanced_by(DispatchTimeInterval::Nanoseconds(-600)).uptime_nanos(), 0);
    }

    #[test]
    fn never_and_overflow_saturate_at_distant_future() {
        let t = DispatchTime::from_uptime_nanos(5);
        assert_eq!(t.advanced_by(DispatchTimeInterval::Never), DispatchTime::DISTANT_FUTURE);
        assert_eq!(DispatchTime::DISTANT_FUTURE.advanced_by(DispatchTimeInterval::Seconds(1)), DispatchTime::DISTANT_FUTURE);
        let near = DispatchTime::from_uptime_nanos(u64::MAX - 1);
        assert_eq!(near.advanced_by(DispatchTimeInterval::Nanoseconds(10)), DispatchTime::DISTANT_FUTURE);
    }

    #[test]
    fn distance_is_signed_and_distant_future_is_never() {
        let a = DispatchTime::from_uptime_nanos(1_000);
        let b = DispatchTime::from_uptime_nanos(3_500);
        assert_eq!(a.distance_to(b), DispatchTimeInterval::Nanoseconds(2_500));
        assert_eq!(b.distance_to(a), DispatchTimeInterval::Nanoseconds(-2_500));
        assert_eq!(a.distance_to(DispatchTime::DISTANT_FUTURE), DispatchTimeInterval::Never);
    }

    #[test]
    fn a_deadline_becomes_a_delay_for_the_queue() {
        let now = DispatchTime::from_uptime_nanos(500_000);
        let deadline = DispatchTime::from_uptime_nanos(2_000_000);
        assert_eq!(deadline.duration_from(now), Duration::from_nanos(1_500_000));
        assert_eq!(now.duration_from(deadline), Duration::ZERO);
    }

    #[test]
    fn wall_time_keeps_microseconds_and_truncates_nanoseconds() {
        let w = DispatchWallTime::from_timespec(1, 500_000_123);
        assert_eq!(w.unix_micros(), 1_500_000);
        assert_eq!((w + DispatchTimeInterval::Milliseconds(2)).unix_micros(), 1_502_000);
        assert_eq!(DispatchWallTime::from_timespec(0, -1_000).unix_micros(), -1);
        assert_eq!(w.advanced_by(DispatchTimeInterval::Never), DispatchWallTime::DISTANT_FUTURE);
    }

    #[test]
    fn timeout_result_follows_the_finished_flag() {
        assert_eq!(DispatchTimeoutResult::from_finished(false), DispatchTimeoutResult::TimedOut);
        assert!(DispatchTimeoutResult::from_finished(true).is_success());
        assert!(!DispatchTimeoutResult::TimedOut.is_success());
    }
}
