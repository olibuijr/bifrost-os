//! Media time as a rational number: a tick count and a timescale (ticks per second), the way the media framework
//! states a time. The demuxer keeps microseconds and the MP4 sample table keeps milliseconds; this type holds either
//! without rounding and converts between them. A time can also be invalid, indefinite or infinite, and such a time
//! compares as nothing.
//!
//! Conversions round half away from zero. Two times compare by value, so `1/2` and `2/4` are the same time.

use std::cmp::Ordering;

/// The times that are not a number of ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    Invalid,
    Indefinite,
    PositiveInfinity,
    NegativeInfinity,
}

/// A point or span of media time.
#[derive(Clone, Copy, Debug)]
pub enum MediaTime {
    /// `value` ticks at `timescale` ticks per second. The timescale is never zero.
    Number { value: i64, timescale: u32 },
    Special(Special),
}

/// Microseconds per second, the timescale of the demuxer.
const MICROS: u32 = 1_000_000;
/// Milliseconds per second, the timescale of the MP4 sample table.
const MILLIS: u32 = 1_000;

impl MediaTime {
    /// `value` ticks at `timescale` ticks per second. `None` when the timescale is zero.
    pub fn new(value: i64, timescale: u32) -> Option<Self> {
        (timescale != 0).then_some(MediaTime::Number { value, timescale })
    }

    /// Zero seconds.
    pub fn zero() -> Self {
        MediaTime::Number { value: 0, timescale: 1 }
    }

    /// A time given in microseconds.
    pub fn from_micros(micros: i64) -> Self {
        MediaTime::Number { value: micros, timescale: MICROS }
    }

    /// A time given in milliseconds.
    pub fn from_millis(millis: i64) -> Self {
        MediaTime::Number { value: millis, timescale: MILLIS }
    }

    /// True when the time is a number of ticks.
    pub fn is_valid(self) -> bool {
        matches!(self, MediaTime::Number { .. })
    }

    /// The time in microseconds, rounded. `None` for a special time or when the result does not fit.
    pub fn as_micros(self) -> Option<i64> {
        self.rescaled(MICROS)
    }

    /// The time in milliseconds, rounded. `None` for a special time or when the result does not fit.
    pub fn as_millis(self) -> Option<i64> {
        self.rescaled(MILLIS)
    }

    /// The time in seconds. `None` for a special time.
    pub fn seconds(self) -> Option<f64> {
        match self {
            MediaTime::Number { value, timescale } => Some(value as f64 / timescale as f64),
            MediaTime::Special(_) => None,
        }
    }

    /// The same time at another timescale, rounded. `None` for a special time, a zero timescale or an overflow.
    pub fn convert(self, timescale: u32) -> Option<Self> {
        let value = self.rescaled(timescale)?;
        Self::new(value, timescale)
    }

    /// The exact sum of two numbers, at the least common multiple of their timescales. `None` for a special time or
    /// when that timescale or the sum does not fit.
    pub fn checked_add(self, other: MediaTime) -> Option<MediaTime> {
        let (MediaTime::Number { value: a, timescale: ta }, MediaTime::Number { value: b, timescale: tb }) = (self, other) else {
            return None;
        };
        let scale = lcm(ta as u64, tb as u64);
        let scale32 = u32::try_from(scale).ok()?;
        let sum = a as i128 * (scale / ta as u64) as i128 + b as i128 * (scale / tb as u64) as i128;
        Some(MediaTime::Number { value: i64::try_from(sum).ok()?, timescale: scale32 })
    }

    /// Orders two numbers by value. `None` when either time is special.
    pub fn compare(self, other: MediaTime) -> Option<Ordering> {
        let (MediaTime::Number { value: a, timescale: ta }, MediaTime::Number { value: b, timescale: tb }) = (self, other) else {
            return None;
        };
        Some((a as i128 * tb as i128).cmp(&(b as i128 * ta as i128)))
    }

    /// True when both are numbers with the same value, whatever their timescales.
    pub fn same_time(self, other: MediaTime) -> bool {
        self.compare(other) == Some(Ordering::Equal)
    }

    fn rescaled(self, to: u32) -> Option<i64> {
        let MediaTime::Number { value, timescale } = self else { return None };
        if to == 0 {
            return None;
        }
        let num = value as i128 * to as i128;
        let den = timescale as i128;
        let mut q = num / den;
        let r = num % den;
        if 2 * r.abs() >= den {
            q += if num < 0 { -1 } else { 1 };
        }
        i64::try_from(q).ok()
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}

/// A span of media time: a start and a duration of zero or more.
#[derive(Clone, Copy, Debug)]
pub struct TimeRange {
    start: MediaTime,
    duration: MediaTime,
}

impl TimeRange {
    /// A range from `start` lasting `duration`. `None` when either is special or the duration is negative.
    pub fn new(start: MediaTime, duration: MediaTime) -> Option<Self> {
        let negative = duration.compare(MediaTime::zero()) == Some(Ordering::Less);
        (start.is_valid() && duration.is_valid() && !negative).then_some(TimeRange { start, duration })
    }

    pub fn start(&self) -> MediaTime {
        self.start
    }

    pub fn duration(&self) -> MediaTime {
        self.duration
    }

    /// The first time after the range. `None` when the sum does not fit.
    pub fn end(&self) -> Option<MediaTime> {
        self.start.checked_add(self.duration)
    }

    /// True when the range lasts no time at all.
    pub fn is_empty(&self) -> bool {
        self.duration.same_time(MediaTime::zero())
    }

    /// True when `time` is at or after the start and before the end.
    pub fn contains(&self, time: MediaTime) -> bool {
        let Some(end) = self.end() else { return false };
        matches!(time.compare(self.start), Some(Ordering::Greater | Ordering::Equal))
            && time.compare(end) == Some(Ordering::Less)
    }

    /// True when the two ranges share some time. An empty range shares none.
    pub fn overlaps(&self, other: &TimeRange) -> bool {
        let (Some(end), Some(other_end)) = (self.end(), other.end()) else { return false };
        !self.is_empty()
            && !other.is_empty()
            && self.start.compare(other_end) == Some(Ordering::Less)
            && other.start.compare(end) == Some(Ordering::Less)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_timescale_is_not_a_time() {
        assert!(MediaTime::new(5, 0).is_none());
        assert!(MediaTime::new(5, 90_000).is_some());
    }

    #[test]
    fn microseconds_and_milliseconds_convert_exactly() {
        assert_eq!(MediaTime::from_micros(1_500_000).as_millis(), Some(1_500));
        assert_eq!(MediaTime::from_millis(2_250).as_micros(), Some(2_250_000));
        assert_eq!(MediaTime::from_millis(2_250).seconds(), Some(2.25));
    }

    #[test]
    fn a_conversion_rounds_half_away_from_zero() {
        // 90000 ticks per second: 45 ticks is 0.5 ms, which rounds to 1 ms; -45 ticks rounds to -1 ms.
        let half = MediaTime::new(45, 90_000).unwrap();
        assert_eq!(half.as_millis(), Some(1));
        assert_eq!(MediaTime::new(-45, 90_000).unwrap().as_millis(), Some(-1));
        assert_eq!(MediaTime::new(44, 90_000).unwrap().as_millis(), Some(0));
    }

    #[test]
    fn a_time_converts_to_another_timescale_by_its_value() {
        let t = MediaTime::new(3, 2).unwrap();
        assert!(t.convert(10).unwrap().same_time(MediaTime::new(15, 10).unwrap()));
        assert!(t.convert(0).is_none());
    }

    #[test]
    fn equal_values_at_different_timescales_are_the_same_time() {
        let a = MediaTime::new(1, 2).unwrap();
        let b = MediaTime::new(2, 4).unwrap();
        assert!(a.same_time(b));
        assert!(!a.same_time(MediaTime::new(1, 3).unwrap()));
    }

    #[test]
    fn a_special_time_is_not_a_number_and_compares_as_nothing() {
        let inf = MediaTime::Special(Special::PositiveInfinity);
        assert!(!inf.is_valid());
        assert_eq!(inf.as_micros(), None);
        assert_eq!(inf.seconds(), None);
        assert_eq!(inf.compare(MediaTime::zero()), None);
        assert!(!MediaTime::Special(Special::Invalid).same_time(MediaTime::Special(Special::Invalid)));
    }

    #[test]
    fn addition_is_exact_at_the_common_timescale() {
        let sum = MediaTime::new(1, 3).unwrap().checked_add(MediaTime::new(1, 2).unwrap()).unwrap();
        assert!(sum.same_time(MediaTime::new(5, 6).unwrap()));
        assert_eq!(sum.convert(6).unwrap().as_micros(), Some(833_333));
    }

    #[test]
    fn addition_refuses_a_special_time_and_an_overflowing_sum() {
        let max = MediaTime::new(i64::MAX, 1).unwrap();
        assert!(max.checked_add(MediaTime::new(1, 1).unwrap()).is_none());
        assert!(MediaTime::zero().checked_add(MediaTime::Special(Special::Indefinite)).is_none());
    }

    #[test]
    fn a_range_holds_its_start_and_not_its_end() {
        let range = TimeRange::new(MediaTime::from_millis(1_000), MediaTime::from_millis(500)).unwrap();
        assert!(range.contains(MediaTime::from_millis(1_000)));
        assert!(range.contains(MediaTime::from_millis(1_499)));
        assert!(!range.contains(MediaTime::from_millis(1_500)));
        assert!(!range.contains(MediaTime::from_millis(999)));
        assert!(range.end().unwrap().same_time(MediaTime::from_millis(1_500)));
    }

    #[test]
    fn a_range_needs_valid_times_and_a_duration_of_zero_or_more() {
        assert!(TimeRange::new(MediaTime::zero(), MediaTime::from_millis(-1)).is_none());
        assert!(TimeRange::new(MediaTime::Special(Special::Indefinite), MediaTime::zero()).is_none());
        assert!(TimeRange::new(MediaTime::zero(), MediaTime::zero()).unwrap().is_empty());
    }

    #[test]
    fn two_ranges_overlap_only_when_they_share_a_time() {
        let a = TimeRange::new(MediaTime::from_millis(0), MediaTime::from_millis(100)).unwrap();
        let touching = TimeRange::new(MediaTime::from_millis(100), MediaTime::from_millis(50)).unwrap();
        let inside = TimeRange::new(MediaTime::from_millis(40), MediaTime::from_millis(10)).unwrap();
        let empty = TimeRange::new(MediaTime::from_millis(50), MediaTime::zero()).unwrap();
        assert!(!a.overlaps(&touching));
        assert!(a.overlaps(&inside));
        assert!(!a.overlaps(&empty));
    }
}
