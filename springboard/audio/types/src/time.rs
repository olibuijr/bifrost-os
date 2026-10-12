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
    /// The host time at which sample `sample_time` plays, extrapolated from this timestamp at `nominal_rate`.
    pub fn host_ns_of(&self, sample_time: f64, nominal_rate: f64) -> u64 {
        let _ = (sample_time, nominal_rate);
        todo!("audio-01")
    }

    /// The sample time at host time `host_ns`, extrapolated from this timestamp.
    pub fn sample_time_at(&self, host_ns: u64, nominal_rate: f64) -> f64 {
        let _ = (host_ns, nominal_rate);
        todo!("audio-01")
    }
}
