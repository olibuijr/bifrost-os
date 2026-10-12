//! MIDI timestamps on the audio host clock (`CLOCK_MONOTONIC_RAW` nanoseconds, the clock of
//! `bifrost_audio_types::TimeStamp::host_ns`). An event stamped with host time maps to a frame offset inside an audio
//! cycle, so a unit gets sample-accurate MIDI.
//!
//! This crate reads no clock itself (the daemon does, with `clock_gettime`); every function takes the time as an
//! argument, which keeps the tests deterministic.

use bifrost_audio_types::TimeStamp;

/// Maps host time to sample time through one anchor timestamp of a device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostClock {
    /// Sample time of the anchor.
    pub anchor_sample: f64,
    /// Host time of the anchor in nanoseconds.
    pub anchor_host_ns: u64,
    /// Nominal sample rate in hertz.
    pub rate: f64,
    /// Actual rate over nominal (1.0 when unknown).
    pub rate_scalar: f64,
}

impl HostClock {
    pub fn new(anchor_sample: f64, anchor_host_ns: u64, rate: f64, rate_scalar: f64) -> HostClock {
        HostClock { anchor_sample, anchor_host_ns, rate, rate_scalar: if rate_scalar > 0.0 { rate_scalar } else { 1.0 } }
    }

    /// From a device timestamp (its `sample_time` and `host_ns` fields).
    pub fn from_timestamp(ts: &TimeStamp, rate: f64) -> HostClock {
        HostClock::new(ts.sample_time, ts.host_ns, rate, ts.rate_scalar)
    }

    fn samples_per_ns(&self) -> f64 {
        self.rate * self.rate_scalar / 1e9
    }

    /// The (fractional) sample time at host time `host_ns`.
    pub fn sample_at(&self, host_ns: u64) -> f64 {
        let dt = host_ns as f64 - self.anchor_host_ns as f64;
        self.anchor_sample + dt * self.samples_per_ns()
    }

    /// The host time at which sample `sample` plays.
    pub fn host_at(&self, sample: f64) -> u64 {
        let dt = (sample - self.anchor_sample) / self.samples_per_ns();
        (self.anchor_host_ns as f64 + dt).round().max(0.0) as u64
    }

    /// The frame inside a cycle of `frames` frames starting at sample `cycle_start` where an event stamped
    /// `host_ns` falls. `None` when the event is for a later cycle; an event that is already late lands on frame 0
    /// so it is played as soon as possible and keeps its order.
    pub fn frame_in_cycle(&self, host_ns: u64, cycle_start: f64, frames: u32) -> Option<u32> {
        let off = self.sample_at(host_ns) - cycle_start;
        if off >= frames as f64 {
            None
        } else if off <= 0.0 {
            Some(0)
        } else {
            Some(off.floor() as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_sample_time_round_trip() {
        // Anchor: sample 1000 at 5 s; 48 kHz, so 1 ms is 48 samples.
        let c = HostClock::new(1000.0, 5_000_000_000, 48_000.0, 1.0);
        assert_eq!(c.sample_at(5_001_000_000), 1048.0);
        assert_eq!(c.sample_at(5_000_000_000), 1000.0);
        assert_eq!(c.host_at(1048.0), 5_001_000_000);
        // A zero rate scalar means "unknown" and is treated as 1.
        assert_eq!(HostClock::new(0.0, 0, 48_000.0, 0.0).rate_scalar, 1.0);
    }

    #[test]
    fn rate_scalar_stretches_time() {
        let c = HostClock::new(0.0, 0, 48_000.0, 1.0001);
        // 1 s at 48 kHz * 1.0001 = 48004.8 samples.
        assert!((c.sample_at(1_000_000_000) - 48_004.8).abs() < 1e-6);
    }

    #[test]
    fn frame_offset_in_a_64_frame_cycle() {
        let c = HostClock::new(0.0, 0, 48_000.0, 1.0);
        // The cycle covers samples 96..160. Sample 144 is at exactly 3 ms (3_000_000 ns).
        assert_eq!(c.host_at(144.0), 3_000_000);
        assert_eq!(c.frame_in_cycle(3_000_000, 96.0, 64), Some(48));
        // Sample 192 (4 ms) belongs to a later cycle.
        assert_eq!(c.frame_in_cycle(4_000_000, 96.0, 64), None);
        // A late event (1 ms = sample 48) plays on frame 0.
        assert_eq!(c.frame_in_cycle(1_000_000, 96.0, 64), Some(0));
        // The cycle start itself is frame 0.
        assert_eq!(c.frame_in_cycle(2_000_000, 96.0, 64), Some(0));
    }

    #[test]
    fn from_timestamp_uses_the_fields() {
        let ts = TimeStamp { sample_time: 480.0, host_ns: 10_000_000, rate_scalar: 1.0, ..Default::default() };
        let c = HostClock::from_timestamp(&ts, 48_000.0);
        assert_eq!(c.anchor_sample, 480.0);
        assert_eq!(c.sample_at(11_000_000), 528.0);
    }
}
