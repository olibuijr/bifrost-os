//! Timestamps: sample time and host time (`CLOCK_MONOTONIC_RAW` nanoseconds, the clock the window server uses for
//! presentation) with the rate scalar. Owner: task 01.

/// Which fields of a timestamp are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TimeStampFlags(pub u32);

impl TimeStampFlags {
    pub const SAMPLE_TIME: TimeStampFlags = TimeStampFlags(1);
    pub const HOST_TIME: TimeStampFlags = TimeStampFlags(2);
    pub const RATE_SCALAR: TimeStampFlags = TimeStampFlags(4);
    pub const WORD_CLOCK: TimeStampFlags = TimeStampFlags(8);

    pub const fn contains(self, other: TimeStampFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for TimeStampFlags {
    type Output = TimeStampFlags;
    fn bitor(self, rhs: TimeStampFlags) -> TimeStampFlags {
        TimeStampFlags(self.0 | rhs.0)
    }
}

/// A point on a device's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TimeStamp {
    pub sample_time: f64,
    pub host_ns: u64,
    /// Actual rate divided by the nominal rate (1.0 at the nominal rate).
    pub rate_scalar: f64,
    pub word_clock: u64,
    pub flags: TimeStampFlags,
}

impl TimeStamp {
    /// The rate scalar, 1.0 when it is unset (0) or invalid.
    fn scalar(&self) -> f64 {
        if self.rate_scalar > 0.0 {
            self.rate_scalar
        } else {
            1.0
        }
    }

    /// The host time at which sample `sample_time` plays, extrapolated from this timestamp at `nominal_rate`.
    pub fn host_ns_of(&self, sample_time: f64, nominal_rate: f64) -> u64 {
        let scalar = self.scalar();
        let dt = (sample_time - self.sample_time) / (nominal_rate * scalar) * 1e9;
        let ns = self.host_ns as f64 + dt;
        if ns <= 0.0 {
            0
        } else {
            (ns + 0.5) as u64
        }
    }

    /// The sample time at host time `host_ns`, extrapolated from this timestamp.
    pub fn sample_time_at(&self, host_ns: u64, nominal_rate: f64) -> f64 {
        let dt = (host_ns as f64 - self.host_ns as f64) / 1e9;
        self.sample_time + dt * nominal_rate * self.scalar()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(scalar: f64) -> TimeStamp {
        TimeStamp {
            sample_time: 1000.0,
            host_ns: 5_000_000_000,
            rate_scalar: scalar,
            word_clock: 0,
            flags: TimeStampFlags::SAMPLE_TIME | TimeStampFlags::HOST_TIME,
        }
    }

    #[test]
    fn extrapolates_at_nominal_rate() {
        let t = ts(1.0);
        // 48000 frames later is exactly one second later.
        assert_eq!(t.host_ns_of(49000.0, 48000.0), 6_000_000_000);
        assert_eq!(t.sample_time_at(5_500_000_000, 48000.0), 25000.0);
        // before the anchor
        assert_eq!(t.host_ns_of(520.0, 48000.0), 4_990_000_000);
    }

    #[test]
    fn rate_scalar_stretches_time() {
        // running 2x fast: 48000 frames take half a second.
        let t = ts(2.0);
        assert_eq!(t.host_ns_of(49000.0, 48000.0), 5_500_000_000);
        assert_eq!(t.sample_time_at(5_500_000_000, 48000.0), 49000.0);
    }

    #[test]
    fn unset_scalar_is_one() {
        let t = ts(0.0);
        assert_eq!(t.host_ns_of(49000.0, 48000.0), 6_000_000_000);
        assert!(t.flags.contains(TimeStampFlags::HOST_TIME));
        assert!(!t.flags.contains(TimeStampFlags::WORD_CLOCK));
    }
}
