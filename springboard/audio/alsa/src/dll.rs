//! The delay-locked loop that tracks the hardware sample clock against the host clock.
//!
//! Input: pairs `(hw_frames, host_ns)` read from the mmapped status page. The loop is the classic second-order
//! DLL (the same shape JACK and PipeWire use): with `w = 2 pi bw dt`, `b = sqrt(2) w` and `c = w^2`, each update
//! moves the time origin by `b e` and the nanoseconds-per-frame estimate by `c e / period`, where `e` is the
//! difference between the measured and the predicted host time of the pair. The bandwidth starts wide and decays
//! exponentially to a narrow floor, so the loop locks within a second and then rejects the timestamp jitter.
//!
//! Outputs: `rate_scalar()` (hardware rate over the nominal rate, > 1 when the hardware runs fast) and
//! `host_time_of(frames)` (the predicted host time of a sample position). No allocation, no syscall.

/// Loop bandwidth at start, in Hz.
pub const BW_WIDE_HZ: f64 = 2.0;
/// Loop bandwidth floor, in Hz.
pub const BW_NARROW_HZ: f64 = 0.2;
/// Time constant of the bandwidth decay, in seconds.
pub const BW_DECAY_S: f64 = 1.0;

#[derive(Debug, Clone)]
pub struct Dll {
    nominal_rate: f64,
    /// Frames between two updates in the nominal case (the quantum).
    period: f64,
    /// Seconds between two updates at the nominal rate.
    dt: f64,
    bw: f64,
    bw_narrow: f64,
    decay_s: f64,
    /// Host time origin (ns, `CLOCK_MONOTONIC_RAW`) the f64 times are relative to.
    base_ns: u64,
    /// Predicted host time (ns after `base_ns`) of sample position `s0`.
    t0: f64,
    s0: u64,
    /// Nanoseconds per frame, estimated.
    npf: f64,
    nominal_npf: f64,
    updates: u64,
}

impl Dll {
    /// A loop for a device at `rate` Hz that is updated about every `period_frames` frames.
    pub fn new(rate: f64, period_frames: u32) -> Dll {
        let period = period_frames.max(1) as f64;
        let nominal_npf = 1e9 / rate;
        Dll {
            nominal_rate: rate,
            period,
            dt: period / rate,
            bw: BW_WIDE_HZ,
            bw_narrow: BW_NARROW_HZ,
            decay_s: BW_DECAY_S,
            base_ns: 0,
            t0: 0.0,
            s0: 0,
            npf: nominal_npf,
            nominal_npf,
            updates: 0,
        }
    }

    /// Restarts the loop at a known pair (the stream start). Bandwidth returns to wide, the rate to nominal.
    pub fn reset(&mut self, frames: u64, host_ns: u64) {
        self.bw = BW_WIDE_HZ;
        self.base_ns = host_ns;
        self.t0 = 0.0;
        self.s0 = frames;
        self.npf = self.nominal_npf;
        self.updates = 0;
    }

    /// Feeds one `(hw_frames, host_ns)` measurement. Pairs must be fed with growing `frames`; the same
    /// position twice is ignored.
    pub fn update(&mut self, frames: u64, host_ns: u64) {
        if frames <= self.s0 && self.updates > 0 {
            return;
        }
        let t = host_ns.wrapping_sub(self.base_ns) as i64 as f64;
        let predicted = self.t0 + (frames as f64 - self.s0 as f64) * self.npf;
        let e = t - predicted;
        let w = 2.0 * std::f64::consts::PI * self.bw * self.dt;
        let b = std::f64::consts::SQRT_2 * w;
        let c = w * w;
        self.t0 = predicted + b * e;
        self.s0 = frames;
        self.npf += c * e / self.period;
        self.bw = (self.bw * (1.0 - self.dt / self.decay_s)).max(self.bw_narrow);
        self.updates += 1;
    }

    /// Hardware rate over nominal rate: 1.0003 for a clock that runs 300 ppm fast.
    pub fn rate_scalar(&self) -> f64 {
        self.nominal_npf / self.npf
    }

    /// The estimated hardware rate in Hz.
    pub fn rate_hz(&self) -> f64 {
        self.nominal_rate * self.rate_scalar()
    }

    /// Predicted host time (ns) at which sample position `frames` is played or captured.
    pub fn host_time_of(&self, frames: u64) -> u64 {
        let rel = self.t0 + (frames as f64 - self.s0 as f64) * self.npf;
        (self.base_ns as f64 + rel).max(0.0) as u64
    }

    /// Sample position the hardware has reached at host time `host_ns`.
    pub fn frames_at(&self, host_ns: u64) -> f64 {
        let rel = host_ns as f64 - self.base_ns as f64;
        self.s0 as f64 + (rel - self.t0) / self.npf
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// The current loop bandwidth in Hz (wide at start, narrow when locked).
    pub fn bandwidth(&self) -> f64 {
        self.bw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulated hardware: `quantum` frames per update, the sample clock running `ppm` fast, started at host time
    /// `start_ns`. Returns the DLL after `seconds`, checking the estimate on the way.
    fn simulate(ppm: f64, seconds: f64, mut check: impl FnMut(f64, &Dll)) -> Dll {
        let (rate, q) = (48_000.0, 64u32);
        let true_npf = 1e9 / rate / (1.0 + ppm * 1e-6);
        let start = 5_000_000_000u64;
        let mut dll = Dll::new(rate, q);
        dll.reset(0, start);
        let n = (seconds * rate / q as f64) as u64;
        for k in 1..=n {
            let frames = k * q as u64;
            dll.update(frames, start + (frames as f64 * true_npf).round() as u64);
            check(frames as f64 / rate, &dll);
        }
        dll
    }

    fn error_ppm(dll: &Dll, ppm: f64) -> f64 {
        (dll.rate_scalar() - 1.0) * 1e6 - ppm
    }

    #[test]
    fn nominal_clock_stays_at_one() {
        let dll = simulate(0.0, 2.0, |_, d| assert!((d.rate_scalar() - 1.0).abs() < 1e-7));
        assert!((dll.rate_hz() - 48_000.0).abs() < 1e-3);
    }

    #[test]
    fn converges_within_two_seconds_at_plus_300_ppm_and_stays() {
        simulate(300.0, 6.0, |t, d| {
            if t >= 2.0 {
                assert!(error_ppm(d, 300.0).abs() < 1.0, "t={t} err={}", error_ppm(d, 300.0));
            }
        });
    }

    #[test]
    fn converges_within_two_seconds_at_minus_300_ppm_and_stays() {
        simulate(-300.0, 6.0, |t, d| {
            if t >= 2.0 {
                assert!(error_ppm(d, -300.0).abs() < 1.0, "t={t} err={}", error_ppm(d, -300.0));
            }
        });
    }

    #[test]
    fn bandwidth_narrows_from_wide_to_the_floor() {
        let mut d = Dll::new(48_000.0, 64);
        assert_eq!(d.bandwidth(), BW_WIDE_HZ);
        d.reset(0, 0);
        for k in 1..=5000u64 {
            d.update(k * 64, k * 64 * 20_833);
        }
        assert_eq!(d.bandwidth(), BW_NARROW_HZ);
    }

    #[test]
    fn host_time_prediction_follows_the_hardware() {
        let dll = simulate(300.0, 6.0, |_, _| {});
        // 10 s of samples on a clock 300 ppm fast take 10 / 1.0003 s of host time
        let want = 5_000_000_000.0 + 480_000.0 * 1e9 / 48_000.0 / 1.0003;
        let got = dll.host_time_of(480_000) as f64;
        assert!((got - want).abs() < 2_000.0, "{got} vs {want}");
        // and the inverse
        let frames = dll.frames_at(got as u64);
        assert!((frames - 480_000.0).abs() < 1.0);
    }

    #[test]
    fn a_repeated_position_is_ignored() {
        let mut d = Dll::new(48_000.0, 64);
        d.reset(0, 1_000);
        d.update(64, 1_000 + 1_333_333);
        let n = d.updates();
        d.update(64, 1_000 + 9_999_999);
        assert_eq!(d.updates(), n);
    }
}
