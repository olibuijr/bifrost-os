//! Hardware audio clock model: the clock an audio device exposes, with its identity, clock domain, nominal sample
//! rate, the rates it accepts and the latency it adds. It also maps sample time to host time and back. The map starts
//! from the first observed (sample, host time) pair; the latest pair anchors the conversion, and the rate measured
//! between the two sets the slope, so a device running slightly off its nominal rate keeps its mapping. Clocks in the
//! same clock domain can be synchronized in hardware. This is a pure model: it reads no clock, opens no device and
//! blocks nothing.

use std::fmt;

/// An inclusive range of sample rates, in hertz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateRange {
    pub minimum: f64,
    pub maximum: f64,
}

impl RateRange {
    /// Whether `rate` lies in the range. Both ends are included.
    pub fn contains(&self, rate: f64) -> bool {
        rate >= self.minimum && rate <= self.maximum
    }
}

/// Why a clock refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    /// The clock is no longer alive, so its rate cannot change.
    NotAlive,
    /// No accepted range contains the requested rate.
    RateNotSupported,
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClockError::NotAlive => write!(f, "the clock is not alive"),
            ClockError::RateNotSupported => write!(f, "the sample rate is not supported"),
        }
    }
}

/// A (sample time, host time in nanoseconds) pair reported by the device.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pair {
    sample: i64,
    host_ns: u64,
}

/// One audio clock.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareClock {
    uid: String,
    clock_domain: u32,
    alive: bool,
    running: bool,
    nominal_rate: f64,
    rates: Vec<RateRange>,
    input_latency: u32,
    output_latency: u32,
    first: Option<Pair>,
    last: Option<Pair>,
}

impl HardwareClock {
    /// A living, stopped clock with the given identifier, nominal rate and accepted rate ranges.
    pub fn new(uid: &str, nominal_rate: f64, rates: Vec<RateRange>) -> Self {
        HardwareClock {
            uid: uid.to_string(),
            clock_domain: 0,
            alive: true,
            running: false,
            nominal_rate,
            rates,
            input_latency: 0,
            output_latency: 0,
            first: None,
            last: None,
        }
    }

    /// The persistent identifier of the clock.
    pub fn uid(&self) -> &str {
        &self.uid
    }

    /// The clock domain. Clocks with the same domain can be synchronized in hardware.
    pub fn clock_domain(&self) -> u32 {
        self.clock_domain
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Whether the clock is providing timestamps.
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn nominal_sample_rate(&self) -> f64 {
        self.nominal_rate
    }

    /// The ranges the nominal rate may be set to.
    pub fn available_nominal_sample_rates(&self) -> &[RateRange] {
        &self.rates
    }

    /// Input latency, in frames.
    pub fn input_latency(&self) -> u32 {
        self.input_latency
    }

    /// Output latency, in frames.
    pub fn output_latency(&self) -> u32 {
        self.output_latency
    }

    pub fn set_clock_domain(&mut self, domain: u32) {
        self.clock_domain = domain;
    }

    pub fn set_latency(&mut self, input: u32, output: u32) {
        self.input_latency = input;
        self.output_latency = output;
    }

    /// Marks the device as gone, or back. A clock that is not alive stops and cannot start again.
    pub fn set_alive(&mut self, alive: bool) {
        self.alive = alive;
        if !alive {
            self.stop();
        }
    }

    /// Starts the timestamps. Returns false when the clock is not alive. The map stays empty until the first
    /// observation.
    pub fn start(&mut self) -> bool {
        if !self.alive {
            return false;
        }
        self.running = true;
        true
    }

    /// Stops the timestamps and forgets the observations.
    pub fn stop(&mut self) {
        self.running = false;
        self.first = None;
        self.last = None;
    }

    /// Changes the nominal rate. The new rate must lie in one of the accepted ranges.
    pub fn set_nominal_sample_rate(&mut self, rate: f64) -> Result<(), ClockError> {
        if !self.alive {
            return Err(ClockError::NotAlive);
        }
        if !self.rates.iter().any(|r| r.contains(rate)) {
            return Err(ClockError::RateNotSupported);
        }
        self.nominal_rate = rate;
        Ok(())
    }

    /// Records a timestamp from the device. Returns false, and keeps the previous pair, when the clock is not running
    /// or when the pair goes back in sample time or in host time.
    pub fn observe(&mut self, sample: i64, host_ns: u64) -> bool {
        if !self.running {
            return false;
        }
        if let Some(last) = self.last {
            if sample < last.sample || host_ns < last.host_ns {
                return false;
            }
        }
        let pair = Pair { sample, host_ns };
        if self.first.is_none() {
            self.first = Some(pair);
        }
        self.last = Some(pair);
        true
    }

    /// The rate used by the mapping: measured between the first and the latest observation when they differ in both
    /// sample and host time, otherwise the nominal rate.
    pub fn rate_in_use(&self) -> f64 {
        match (self.first, self.last) {
            (Some(f), Some(l)) if l.sample > f.sample && l.host_ns > f.host_ns => {
                (l.sample - f.sample) as f64 * 1_000_000_000.0 / (l.host_ns - f.host_ns) as f64
            }
            _ => self.nominal_rate,
        }
    }

    /// The host time, in nanoseconds, at which `sample` falls. None when the clock is not running, has no
    /// observation, or the time would be before the start of host time.
    pub fn sample_to_host(&self, sample: i64) -> Option<u64> {
        if !self.running {
            return None;
        }
        let anchor = self.last?;
        let rate = self.rate_in_use();
        if rate <= 0.0 {
            return None;
        }
        let dt = sample.saturating_sub(anchor.sample) as f64 * 1_000_000_000.0 / rate;
        let host = anchor.host_ns as f64 + dt;
        if host < 0.0 {
            return None;
        }
        Some(host.round() as u64)
    }

    /// The sample time that falls at host time `host_ns`, rounded to the nearest sample. None under the same
    /// conditions as [`HardwareClock::sample_to_host`].
    pub fn host_to_sample(&self, host_ns: u64) -> Option<i64> {
        if !self.running {
            return None;
        }
        let anchor = self.last?;
        let rate = self.rate_in_use();
        if rate <= 0.0 {
            return None;
        }
        let dt = host_ns as f64 - anchor.host_ns as f64;
        let sample = anchor.sample as f64 + dt * rate / 1_000_000_000.0;
        Some(sample.round() as i64)
    }
}

/// The clocks in `clocks` that can be synchronized in hardware with `clock`: the living ones in the same clock domain,
/// leaving out `clock` itself, in list order.
pub fn sync_peers<'a>(clock: &HardwareClock, clocks: &'a [HardwareClock]) -> Vec<&'a HardwareClock> {
    clocks
        .iter()
        .filter(|c| c.alive && c.clock_domain == clock.clock_domain && c.uid != clock.uid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(uid: &str) -> HardwareClock {
        HardwareClock::new(uid, 48_000.0, vec![RateRange { minimum: 8_000.0, maximum: 96_000.0 }])
    }

    #[test]
    fn rate_must_lie_in_an_accepted_range_and_a_dead_clock_refuses_it() {
        let mut c = clock("a");
        assert_eq!(c.set_nominal_sample_rate(44_100.0), Ok(()));
        assert_eq!(c.nominal_sample_rate(), 44_100.0);
        assert_eq!(c.set_nominal_sample_rate(192_000.0), Err(ClockError::RateNotSupported));
        assert_eq!(c.nominal_sample_rate(), 44_100.0);
        c.set_alive(false);
        assert_eq!(c.set_nominal_sample_rate(48_000.0), Err(ClockError::NotAlive));
        assert!(!c.start());
    }

    #[test]
    fn host_time_maps_from_the_first_observation_at_the_nominal_rate() {
        let mut c = clock("a");
        assert_eq!(c.sample_to_host(0), None);
        assert!(c.start());
        assert_eq!(c.sample_to_host(0), None);
        assert!(c.observe(0, 1_000_000_000));
        assert_eq!(c.sample_to_host(48_000), Some(2_000_000_000));
        assert_eq!(c.host_to_sample(1_500_000_000), Some(24_000));
        c.stop();
        assert_eq!(c.host_to_sample(1_500_000_000), None);
    }

    #[test]
    fn two_observations_measure_the_rate_the_device_runs_at() {
        let mut c = clock("a");
        assert!(c.start());
        assert!(c.observe(0, 0));
        assert!(c.observe(48_048, 1_000_000_000));
        assert_eq!(c.rate_in_use(), 48_048.0);
        assert_eq!(c.sample_to_host(96_096), Some(2_000_000_000));
        assert_eq!(c.host_to_sample(2_000_000_000), Some(96_096));
    }

    #[test]
    fn observations_must_not_go_back_in_sample_or_host_time() {
        let mut c = clock("a");
        assert!(!c.observe(0, 0));
        assert!(c.start());
        assert!(c.observe(100, 5_000));
        assert!(!c.observe(50, 6_000));
        assert!(!c.observe(200, 4_000));
        assert!(c.observe(200, 6_000));
    }

    #[test]
    fn only_living_clocks_in_the_same_domain_sync_with_each_other() {
        let mut a = clock("a");
        a.set_clock_domain(7);
        let mut b = clock("b");
        b.set_clock_domain(7);
        let mut c = clock("c");
        c.set_clock_domain(9);
        let mut d = clock("d");
        d.set_clock_domain(7);
        d.set_alive(false);
        let all = vec![a.clone(), b, c, d];
        let peers = sync_peers(&a, &all);
        let uids: Vec<&str> = peers.iter().map(|p| p.uid()).collect();
        assert_eq!(uids, vec!["b"]);
    }

    #[test]
    fn latency_domain_and_range_membership_are_reported_as_set() {
        let mut c = clock("a");
        c.set_latency(256, 512);
        c.set_clock_domain(3);
        assert_eq!((c.input_latency(), c.output_latency(), c.clock_domain()), (256, 512, 3));
        let range = c.available_nominal_sample_rates()[0];
        assert!(range.contains(96_000.0));
        assert!(!range.contains(96_000.5));
    }
}
