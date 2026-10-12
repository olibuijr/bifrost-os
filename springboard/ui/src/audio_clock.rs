//! Sample clocks: the nominal sample rate of a clock, the rate ranges it accepts, the start that ties sample
//! position zero to a host time, and the clock domain that says which clocks can be synchronized in hardware. These
//! are pure models of the clock objects a system audio service keeps. They do not read a device; the host time is
//! passed in by the caller, in nanoseconds.

/// The rate a new clock runs at.
pub const DEFAULT_RATE: f64 = 48_000.0;

/// Nanoseconds in one second.
const NANOS: f64 = 1_000_000_000.0;

/// Why a rate or range was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    /// The rate is not a finite number above zero.
    InvalidRate,
    /// The rate lies outside every range the clock accepts.
    OutsideRange,
    /// The range is not finite, its lower bound is not above zero, or its lower bound is above its upper bound.
    InvalidRange,
    /// The clock is running. This model refuses a rate change while running, because it has no way to re-anchor
    /// the sample position.
    Running,
}

/// One clock. Sample positions are only defined while the clock runs, from the host time given to `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleClock {
    uid: String,
    clock_domain: u32,
    nominal_rate: f64,
    ranges: Vec<(f64, f64)>,
    alive: bool,
    running_from: Option<u64>,
}

impl SampleClock {
    /// A clock at `DEFAULT_RATE`, with no rate ranges, alive and stopped.
    pub fn new(uid: &str, clock_domain: u32) -> Self {
        SampleClock {
            uid: uid.into(),
            clock_domain,
            nominal_rate: DEFAULT_RATE,
            ranges: Vec::new(),
            alive: true,
            running_from: None,
        }
    }

    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn clock_domain(&self) -> u32 {
        self.clock_domain
    }

    pub fn nominal_rate(&self) -> f64 {
        self.nominal_rate
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    pub fn is_running(&self) -> bool {
        self.running_from.is_some()
    }

    /// Marks the clock usable or gone. A clock that is not alive cannot be synchronized with another clock.
    pub fn set_alive(&mut self, alive: bool) {
        self.alive = alive;
    }

    /// Adds an accepted rate range, inclusive at both ends. When no range is added, any positive rate is accepted.
    pub fn add_rate_range(&mut self, min: f64, max: f64) -> Result<(), ClockError> {
        if !min.is_finite() || !max.is_finite() || min <= 0.0 || min > max {
            return Err(ClockError::InvalidRange);
        }
        self.ranges.push((min, max));
        Ok(())
    }

    pub fn rate_ranges(&self) -> &[(f64, f64)] {
        &self.ranges
    }

    /// Sets the nominal rate. The rate must be positive and finite, lie in a range when ranges exist, and the clock
    /// must not be running.
    pub fn set_nominal_rate(&mut self, rate: f64) -> Result<(), ClockError> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(ClockError::InvalidRate);
        }
        if self.is_running() {
            return Err(ClockError::Running);
        }
        if !self.ranges.is_empty() && !self.ranges.iter().any(|(lo, hi)| rate >= *lo && rate <= *hi) {
            return Err(ClockError::OutsideRange);
        }
        self.nominal_rate = rate;
        Ok(())
    }

    /// Starts the clock so that sample position zero falls on `host_ns`. Returns false when already running.
    pub fn start(&mut self, host_ns: u64) -> bool {
        if self.is_running() {
            return false;
        }
        self.running_from = Some(host_ns);
        true
    }

    /// Stops the clock and drops its anchor. Returns false when it was not running.
    pub fn stop(&mut self) -> bool {
        self.running_from.take().is_some()
    }

    /// The sample position at `host_ns`. `None` when the clock is stopped or `host_ns` is before the start.
    pub fn sample_time_at(&self, host_ns: u64) -> Option<f64> {
        let start = self.running_from?;
        if host_ns < start {
            return None;
        }
        Some((host_ns - start) as f64 * self.nominal_rate / NANOS)
    }

    /// The host time, in nanoseconds, at which sample position `sample` falls. Rounded to the nearest nanosecond.
    /// `None` when the clock is stopped or the position is negative or not finite.
    pub fn host_time_at(&self, sample: f64) -> Option<u64> {
        let start = self.running_from?;
        if !sample.is_finite() || sample < 0.0 {
            return None;
        }
        Some(start + (sample * NANOS / self.nominal_rate).round() as u64)
    }

    /// True when both clocks are alive and in the same clock domain, so they can be synchronized in hardware.
    pub fn can_sync_with(&self, other: &SampleClock) -> bool {
        self.alive && other.alive && self.clock_domain == other.clock_domain
    }
}

/// The uid of the clock that should drive a group: the first clock that is alive and running. `None` when no clock
/// runs. The choice order is this model's own rule.
pub fn suggested_reference(clocks: &[SampleClock]) -> Option<&str> {
    clocks.iter().find(|c| c.alive && c.is_running()).map(|c| c.uid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_clock_runs_at_the_default_rate_and_has_no_sample_time() {
        let clock = SampleClock::new("clock.a", 1);
        assert_eq!(clock.nominal_rate(), 48_000.0);
        assert!(!clock.is_running());
        assert_eq!(clock.sample_time_at(0), None);
    }

    #[test]
    fn sample_time_follows_host_time_from_the_start() {
        let mut clock = SampleClock::new("clock.a", 1);
        assert!(clock.start(1_000_000_000));
        assert_eq!(clock.sample_time_at(2_000_000_000), Some(48_000.0));
        assert_eq!(clock.sample_time_at(1_500_000_000), Some(24_000.0));
        assert_eq!(clock.sample_time_at(999_999_999), None);
    }

    #[test]
    fn host_time_is_the_inverse_of_sample_time() {
        let mut clock = SampleClock::new("clock.a", 1);
        clock.start(1_000_000_000);
        assert_eq!(clock.host_time_at(24_000.0), Some(1_500_000_000));
        assert_eq!(clock.host_time_at(-1.0), None);
        assert_eq!(clock.host_time_at(f64::NAN), None);
    }

    #[test]
    fn the_rate_must_be_positive_and_inside_a_range() {
        let mut clock = SampleClock::new("clock.a", 1);
        assert_eq!(clock.set_nominal_rate(0.0), Err(ClockError::InvalidRate));
        assert_eq!(clock.set_nominal_rate(f64::NAN), Err(ClockError::InvalidRate));
        assert_eq!(clock.add_rate_range(48_000.0, 44_100.0), Err(ClockError::InvalidRange));
        clock.add_rate_range(44_100.0, 48_000.0).unwrap();
        assert_eq!(clock.set_nominal_rate(96_000.0), Err(ClockError::OutsideRange));
        clock.set_nominal_rate(44_100.0).unwrap();
        assert_eq!(clock.nominal_rate(), 44_100.0);
    }

    #[test]
    fn a_running_clock_refuses_a_rate_change_until_it_stops() {
        let mut clock = SampleClock::new("clock.a", 1);
        assert!(clock.start(0));
        assert!(!clock.start(5), "a second start is refused");
        assert_eq!(clock.sample_time_at(0), Some(0.0));
        assert_eq!(clock.set_nominal_rate(44_100.0), Err(ClockError::Running));
        assert!(clock.stop());
        assert!(!clock.stop(), "a second stop is refused");
        assert_eq!(clock.sample_time_at(10), None);
        clock.set_nominal_rate(44_100.0).unwrap();
    }

    #[test]
    fn clocks_sync_only_in_the_same_domain_and_while_alive() {
        let a = SampleClock::new("clock.a", 7);
        let b = SampleClock::new("clock.b", 7);
        let c = SampleClock::new("clock.c", 8);
        assert!(a.can_sync_with(&b));
        assert!(!a.can_sync_with(&c));
        let mut dead = SampleClock::new("clock.d", 7);
        dead.set_alive(false);
        assert!(!a.can_sync_with(&dead));
    }

    #[test]
    fn the_suggested_reference_is_the_first_alive_running_clock() {
        let a = SampleClock::new("clock.a", 1);
        let mut b = SampleClock::new("clock.b", 1);
        let mut c = SampleClock::new("clock.c", 1);
        b.start(0);
        c.start(0);
        b.set_alive(false);
        assert_eq!(suggested_reference(&[a.clone(), b, c]), Some("clock.c"));
        assert_eq!(suggested_reference(&[a]), None);
    }
}
