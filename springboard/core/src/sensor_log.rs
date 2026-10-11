//! Recorded sensor samples: a bounded, time-ordered log that a caller queries by time range.
//!
//! The log keeps the newest `capacity` samples and drops the oldest when it is full. Times are seconds on the
//! caller's clock, the same clock [`crate::motion::MotionEstimator`] takes. Samples must arrive in time order;
//! a sample that goes back in time is refused. The log has no thread, no file and no clock of its own.
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct SensorLog<T> {
    samples: VecDeque<(f64, T)>,
    capacity: usize,
}

impl<T: Clone> SensorLog<T> {
    /// A log that keeps at most `capacity` samples (at least one).
    pub fn new(capacity: usize) -> Self {
        SensorLog { samples: VecDeque::new(), capacity: capacity.max(1) }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Appends a sample taken at `t`. Returns `false`, and leaves the log as it was, when `t` is not finite or
    /// is earlier than the newest sample. A full log drops its oldest sample first.
    pub fn record(&mut self, t: f64, sample: T) -> bool {
        if !t.is_finite() {
            return false;
        }
        if let Some((newest, _)) = self.samples.back() {
            if t < *newest {
                return false;
            }
        }
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back((t, sample));
        true
    }

    /// The samples taken from `from` to `to`, both ends included, oldest first. Empty when `from` is after `to`.
    pub fn between(&self, from: f64, to: f64) -> Vec<(f64, T)> {
        if !(from <= to) {
            return Vec::new();
        }
        let start = self.samples.partition_point(|(t, _)| *t < from);
        self.samples.range(start..).take_while(|(t, _)| *t <= to).cloned().collect()
    }

    /// Drops every sample taken before `t`.
    pub fn discard_before(&mut self, t: f64) {
        let n = self.samples.partition_point(|(s, _)| *s < t);
        self.samples.drain(..n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_log_keeps_the_newest_samples() {
        let mut log = SensorLog::new(3);
        for i in 0..5 {
            assert!(log.record(i as f64, i));
        }
        assert_eq!(log.len(), 3);
        assert_eq!(log.between(f64::MIN, f64::MAX), vec![(2.0, 2), (3.0, 3), (4.0, 4)]);
    }

    #[test]
    fn samples_out_of_time_order_or_without_a_finite_time_are_refused() {
        let mut log = SensorLog::new(10);
        assert!(log.record(5.0, "a"));
        assert!(!log.record(4.0, "late"), "earlier than the newest sample");
        assert!(!log.record(f64::NAN, "nan"));
        assert!(!log.record(f64::INFINITY, "inf"));
        assert!(log.record(5.0, "same time is kept"));
        assert_eq!(log.len(), 2);
        assert_eq!(log.between(0.0, 10.0), vec![(5.0, "a"), (5.0, "same time is kept")]);
    }

    #[test]
    fn between_includes_both_ends_and_is_empty_for_a_backwards_range() {
        let mut log = SensorLog::new(100);
        for i in 0..10 {
            log.record(i as f64 * 0.5, i);
        }
        // 1.0 and 2.5 are samples 2 and 5: both ends are included
        assert_eq!(log.between(1.0, 2.5).iter().map(|s| s.1).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        // a range between two samples holds none
        assert!(log.between(1.1, 1.4).is_empty());
        assert!(log.between(3.0, 1.0).is_empty());
        assert!(log.between(f64::NAN, 1.0).is_empty());
    }

    #[test]
    fn discard_before_drops_only_the_older_samples() {
        let mut log = SensorLog::new(10);
        for i in 0..6 {
            log.record(i as f64, i);
        }
        log.discard_before(3.0);
        assert_eq!(log.len(), 3);
        assert_eq!(log.between(0.0, 10.0).first(), Some(&(3.0, 3)));
        log.discard_before(100.0);
        assert!(log.is_empty());
    }

    #[test]
    fn a_zero_capacity_still_keeps_one_sample() {
        let mut log = SensorLog::new(0);
        assert_eq!(log.capacity(), 1);
        log.record(1.0, 'a');
        log.record(2.0, 'b');
        assert_eq!(log.between(0.0, 3.0), vec![(2.0, 'b')]);
    }
}
