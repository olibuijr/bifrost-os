//! Rational media time: a value over a timescale, with the non-numeric states a media timeline needs (invalid,
//! indefinite, positive and negative infinity), exact arithmetic, conversion between timescales with a rounding
//! method, comparison that works across timescales, and time ranges. This is a pure model of the time type that the
//! media pipeline passes between components. A result that does not fit the 64-bit value becomes invalid rather
//! than wrapping.

use std::cmp::Ordering;
use std::ops::{Add, Sub};

/// Flag bits. A time is valid when `VALID` is set; infinities and the indefinite time are valid but not numeric.
const VALID: u32 = 1;
/// Set when a conversion or a construction from seconds had to round the value.
const ROUNDED: u32 = 2;
const POS_INF: u32 = 4;
const NEG_INF: u32 = 8;
const INDEFINITE: u32 = 16;

/// How a conversion rounds a result that does not land exactly on the new timescale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMethod {
    /// Rounds to the nearest value; a half rounds away from zero. This is the default.
    HalfAwayFromZero,
    /// Rounds any fraction away from zero.
    AwayFromZero,
    /// Drops the fraction.
    TowardZero,
    /// Rounds up toward positive infinity.
    TowardPositiveInfinity,
    /// Rounds down toward negative infinity.
    TowardNegativeInfinity,
}

impl Default for RoundingMethod {
    fn default() -> Self {
        RoundingMethod::HalfAwayFromZero
    }
}

/// A point or an amount on a media timeline: `value / timescale` seconds when numeric.
///
/// Arithmetic between two numeric times with different timescales is done on the larger timescale, with rounding
/// (half away from zero) when an operand does not divide exactly. Adding a non-numeric time follows these rules:
/// an invalid operand gives an invalid result, an indefinite operand gives an indefinite result, and infinities
/// absorb finite values. Positive infinity plus negative infinity is invalid.
#[derive(Clone, Copy, Debug)]
pub struct MediaTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

/// Round `value / from` to the timescale `to` with `method`. Returns the new value and whether it was rounded, or
/// `None` when the new value does not fit in 64 bits. Both timescales must be positive.
fn rescale(value: i64, from: i32, to: i32, method: RoundingMethod) -> Option<(i64, bool)> {
    let num = value as i128 * to as i128;
    let den = from as i128;
    let q = num / den;
    let r = num % den;
    if r == 0 {
        return i64::try_from(q).ok().map(|v| (v, false));
    }
    let sign: i128 = if num < 0 { -1 } else { 1 };
    let rounded = match method {
        RoundingMethod::HalfAwayFromZero => {
            if 2 * r.abs() >= den {
                q + sign
            } else {
                q
            }
        }
        RoundingMethod::AwayFromZero => q + sign,
        RoundingMethod::TowardZero => q,
        RoundingMethod::TowardPositiveInfinity => {
            if num > 0 {
                q + 1
            } else {
                q
            }
        }
        RoundingMethod::TowardNegativeInfinity => {
            if num < 0 {
                q - 1
            } else {
                q
            }
        }
    };
    i64::try_from(rounded).ok().map(|v| (v, true))
}

impl MediaTime {
    fn make(value: i64, timescale: i32, rounded: bool) -> MediaTime {
        let mut flags = VALID;
        if rounded {
            flags |= ROUNDED;
        }
        MediaTime { value, timescale, flags, epoch: 0 }
    }

    /// The time `value / timescale`. A timescale that is not positive gives an invalid time.
    pub fn new(value: i64, timescale: i32) -> MediaTime {
        if timescale <= 0 {
            return MediaTime::invalid();
        }
        MediaTime::make(value, timescale, false)
    }

    /// The same as `new`, with an epoch. The epoch is kept through conversions and is zero otherwise.
    pub fn with_epoch(value: i64, timescale: i32, epoch: i64) -> MediaTime {
        MediaTime { epoch, ..MediaTime::new(value, timescale) }
    }

    /// The time nearest to `seconds` on `preferred_timescale`. NaN or a timescale that is not positive gives an
    /// invalid time, infinities give the matching infinity, and a value that does not fit in 64 bits gives an
    /// invalid time.
    pub fn from_seconds(seconds: f64, preferred_timescale: i32) -> MediaTime {
        if seconds.is_nan() || preferred_timescale <= 0 {
            return MediaTime::invalid();
        }
        if seconds == f64::INFINITY {
            return MediaTime::positive_infinity();
        }
        if seconds == f64::NEG_INFINITY {
            return MediaTime::negative_infinity();
        }
        let scaled = seconds * preferred_timescale as f64;
        let nearest = scaled.round();
        if nearest.abs() >= 9.2e18 {
            return MediaTime::invalid();
        }
        MediaTime::make(nearest as i64, preferred_timescale, scaled != nearest)
    }

    /// A time that is not valid.
    pub fn invalid() -> MediaTime {
        MediaTime { value: 0, timescale: 0, flags: 0, epoch: 0 }
    }

    /// Zero seconds.
    pub fn zero() -> MediaTime {
        MediaTime::make(0, 1, false)
    }

    /// A time that is valid but has no numeric value.
    pub fn indefinite() -> MediaTime {
        MediaTime { value: 0, timescale: 1, flags: VALID | INDEFINITE, epoch: 0 }
    }

    pub fn positive_infinity() -> MediaTime {
        MediaTime { value: 0, timescale: 1, flags: VALID | POS_INF, epoch: 0 }
    }

    pub fn negative_infinity() -> MediaTime {
        MediaTime { value: 0, timescale: 1, flags: VALID | NEG_INF, epoch: 0 }
    }

    /// Positive infinity for a positive sign, negative infinity for a negative sign, zero otherwise.
    fn infinity(sign: i8) -> MediaTime {
        if sign > 0 {
            MediaTime::positive_infinity()
        } else if sign < 0 {
            MediaTime::negative_infinity()
        } else {
            MediaTime::zero()
        }
    }

    /// The numerator of the rational time. Zero for non-numeric times.
    pub fn value(&self) -> i64 {
        self.value
    }

    /// The denominator of the rational time. Zero for invalid times.
    pub fn timescale(&self) -> i32 {
        self.timescale
    }

    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    pub fn is_valid(&self) -> bool {
        (self.flags & VALID) != 0
    }

    /// True when the time is valid and has a finite value.
    pub fn is_numeric(&self) -> bool {
        self.is_valid() && (self.flags & (POS_INF | NEG_INF | INDEFINITE)) == 0
    }

    pub fn is_indefinite(&self) -> bool {
        (self.flags & INDEFINITE) != 0
    }

    pub fn is_positive_infinity(&self) -> bool {
        (self.flags & POS_INF) != 0
    }

    pub fn is_negative_infinity(&self) -> bool {
        (self.flags & NEG_INF) != 0
    }

    /// True when a conversion or a construction from seconds rounded the value.
    pub fn has_been_rounded(&self) -> bool {
        (self.flags & ROUNDED) != 0
    }

    /// The time in seconds. Positive and negative infinity give the float infinities, and invalid and indefinite
    /// times give NaN.
    pub fn seconds(&self) -> f64 {
        if self.is_numeric() {
            self.value as f64 / self.timescale as f64
        } else if self.is_positive_infinity() {
            f64::INFINITY
        } else if self.is_negative_infinity() {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    }

    /// The sign of an infinite time: 1 for positive infinity, -1 for negative infinity, 0 otherwise.
    fn inf_sign(&self) -> i8 {
        if self.is_positive_infinity() {
            1
        } else if self.is_negative_infinity() {
            -1
        } else {
            0
        }
    }

    /// The time with its sign reversed. Invalid and indefinite times are returned unchanged.
    fn negated(self) -> MediaTime {
        if !self.is_valid() || self.is_indefinite() {
            return self;
        }
        if self.is_positive_infinity() {
            return MediaTime::negative_infinity();
        }
        if self.is_negative_infinity() {
            return MediaTime::positive_infinity();
        }
        match self.value.checked_neg() {
            Some(v) => MediaTime { value: v, ..self },
            None => MediaTime::invalid(),
        }
    }

    fn add_numeric(self, other: MediaTime) -> Option<MediaTime> {
        let ts = self.timescale.max(other.timescale);
        let (a, ra) = rescale(self.value, self.timescale, ts, RoundingMethod::HalfAwayFromZero)?;
        let (b, rb) = rescale(other.value, other.timescale, ts, RoundingMethod::HalfAwayFromZero)?;
        let sum = a.checked_add(b)?;
        Some(MediaTime::make(sum, ts, ra || rb))
    }

    /// Converts a numeric time to `new_timescale` with `method`. Non-numeric times are returned unchanged. A
    /// timescale that is not positive gives an invalid time.
    pub fn convert_scale(&self, new_timescale: i32, method: RoundingMethod) -> MediaTime {
        if !self.is_numeric() {
            return *self;
        }
        if new_timescale <= 0 {
            return MediaTime::invalid();
        }
        match rescale(self.value, self.timescale, new_timescale, method) {
            Some((v, rounded)) => {
                let mut out = MediaTime::make(v, new_timescale, rounded || self.has_been_rounded());
                out.epoch = self.epoch;
                out
            }
            None => MediaTime::invalid(),
        }
    }

    /// Orders two times. Returns `None` when either time is invalid or indefinite. Positive infinity is above every
    /// numeric time, negative infinity is below it, and two equal infinities compare equal.
    pub fn compare(&self, other: &MediaTime) -> Option<Ordering> {
        if !self.is_valid() || !other.is_valid() || self.is_indefinite() || other.is_indefinite() {
            return None;
        }
        let (ra, rb) = (self.inf_sign(), other.inf_sign());
        if ra == 0 && rb == 0 {
            let left = self.value as i128 * other.timescale as i128;
            let right = other.value as i128 * self.timescale as i128;
            Some(left.cmp(&right))
        } else if ra != rb {
            Some(ra.cmp(&rb))
        } else {
            Some(Ordering::Equal)
        }
    }
}

impl Add for MediaTime {
    type Output = MediaTime;

    fn add(self, rhs: MediaTime) -> MediaTime {
        if !self.is_valid() || !rhs.is_valid() {
            return MediaTime::invalid();
        }
        if self.is_indefinite() || rhs.is_indefinite() {
            return MediaTime::indefinite();
        }
        let (a, b) = (self.inf_sign(), rhs.inf_sign());
        if a != 0 || b != 0 {
            return match (a, b) {
                (0, s) | (s, 0) => MediaTime::infinity(s),
                (x, y) if x == y => MediaTime::infinity(x),
                _ => MediaTime::invalid(),
            };
        }
        self.add_numeric(rhs).unwrap_or_else(MediaTime::invalid)
    }
}

impl Sub for MediaTime {
    type Output = MediaTime;

    fn sub(self, rhs: MediaTime) -> MediaTime {
        self + rhs.negated()
    }
}

/// A span of media time from a start time for a duration. A range is valid when both its start and its duration
/// are valid. A range whose duration is negative contains no time.
#[derive(Clone, Copy, Debug)]
pub struct TimeRange {
    start: MediaTime,
    duration: MediaTime,
}

impl TimeRange {
    pub fn new(start: MediaTime, duration: MediaTime) -> TimeRange {
        TimeRange { start, duration }
    }

    /// The range from `start` up to `end`. Its duration is `end - start`.
    pub fn from_start_end(start: MediaTime, end: MediaTime) -> TimeRange {
        TimeRange { start, duration: end - start }
    }

    /// A range with no valid start or duration.
    pub fn invalid() -> TimeRange {
        TimeRange { start: MediaTime::invalid(), duration: MediaTime::invalid() }
    }

    /// An empty range at zero.
    pub fn zero() -> TimeRange {
        TimeRange { start: MediaTime::zero(), duration: MediaTime::zero() }
    }

    pub fn start(&self) -> MediaTime {
        self.start
    }

    pub fn duration(&self) -> MediaTime {
        self.duration
    }

    /// The first time after the range: `start + duration`.
    pub fn end(&self) -> MediaTime {
        self.start + self.duration
    }

    pub fn is_valid(&self) -> bool {
        self.start.is_valid() && self.duration.is_valid()
    }

    pub fn is_indefinite(&self) -> bool {
        self.start.is_indefinite() || self.duration.is_indefinite()
    }

    /// True when the duration is valid and zero.
    pub fn is_empty(&self) -> bool {
        self.is_valid() && self.duration.compare(&MediaTime::zero()) == Some(Ordering::Equal)
    }

    /// True when `time` lies at or after the start and before the end.
    pub fn contains_time(&self, time: &MediaTime) -> bool {
        let end = self.end();
        let after_start = matches!(self.start.compare(time), Some(Ordering::Less) | Some(Ordering::Equal));
        after_start && time.compare(&end) == Some(Ordering::Less)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_valid_only_with_a_positive_timescale() {
        let t = MediaTime::new(3, 2);
        assert!(t.is_valid() && t.is_numeric());
        assert_eq!(t.seconds(), 1.5);
        assert!(!MediaTime::new(3, 0).is_valid());
        assert!(!MediaTime::new(3, -1).is_valid());
        assert!(!MediaTime::invalid().is_valid());
        assert!(MediaTime::zero().is_numeric());
    }

    #[test]
    fn seconds_round_to_the_preferred_timescale_and_record_the_rounding() {
        let half = MediaTime::from_seconds(0.5, 2);
        assert_eq!((half.value(), half.timescale()), (1, 2));
        assert!(!half.has_been_rounded());
        // 10 / 3 = 3.33..., which rounds to 3 on a timescale of 10.
        let third = MediaTime::from_seconds(1.0 / 3.0, 10);
        assert_eq!((third.value(), third.timescale()), (3, 10));
        assert!(third.has_been_rounded());
        assert!(!MediaTime::from_seconds(f64::NAN, 10).is_valid());
        assert!(MediaTime::from_seconds(f64::INFINITY, 10).is_positive_infinity());
    }

    #[test]
    fn addition_across_timescales_rounds_half_away_from_zero() {
        // 1/2 on timescale 3 is 1.5, which rounds half away from zero to 2. 1/3 stays 1. The sum is 3/3 = 1 s.
        let sum = MediaTime::new(1, 2) + MediaTime::new(1, 3);
        assert_eq!(sum.seconds(), 1.0);
        assert_eq!(sum.timescale(), 3);
        assert!(sum.has_been_rounded());
        // Same timescale is exact.
        let diff = MediaTime::new(5, 10) - MediaTime::new(1, 10);
        assert_eq!((diff.value(), diff.timescale()), (4, 10));
        assert!(!diff.has_been_rounded());
    }

    #[test]
    fn non_numeric_operands_propagate_as_the_rules_say() {
        let one = MediaTime::new(1, 2);
        assert!(!(MediaTime::invalid() + one).is_valid());
        assert!((MediaTime::indefinite() + one).is_indefinite());
        assert!((MediaTime::positive_infinity() + one).is_positive_infinity());
        assert!((MediaTime::positive_infinity() + MediaTime::positive_infinity()).is_positive_infinity());
        assert!(!(MediaTime::positive_infinity() + MediaTime::negative_infinity()).is_valid());
        assert!((MediaTime::negative_infinity() - one).is_negative_infinity());
    }

    #[test]
    fn comparison_works_across_timescales_and_orders_infinities() {
        use std::cmp::Ordering::*;
        assert_eq!(MediaTime::new(1, 2).compare(&MediaTime::new(2, 4)), Some(Equal));
        assert_eq!(MediaTime::new(1, 3).compare(&MediaTime::new(1, 2)), Some(Less));
        assert_eq!(MediaTime::positive_infinity().compare(&MediaTime::new(9, 1)), Some(Greater));
        assert_eq!(MediaTime::negative_infinity().compare(&MediaTime::new(-9, 1)), Some(Less));
        assert_eq!(MediaTime::invalid().compare(&MediaTime::zero()), None);
    }

    #[test]
    fn conversion_applies_the_rounding_method() {
        // 1/3 on timescale 2 is 0.67: toward zero gives 0 and away from zero gives 1.
        let t = MediaTime::new(1, 3);
        let toward_zero = t.convert_scale(2, RoundingMethod::TowardZero);
        assert_eq!(toward_zero.value(), 0);
        assert!(toward_zero.has_been_rounded());
        assert_eq!(t.convert_scale(2, RoundingMethod::AwayFromZero).value(), 1);
        // -1/2 on timescale 1 is -0.5: half away from zero gives -1, toward positive infinity gives 0.
        let half = MediaTime::new(-1, 2);
        assert_eq!(half.convert_scale(1, RoundingMethod::HalfAwayFromZero).value(), -1);
        assert_eq!(half.convert_scale(1, RoundingMethod::TowardPositiveInfinity).value(), 0);
        assert_eq!(MediaTime::new(1, 2).convert_scale(1, RoundingMethod::HalfAwayFromZero).value(), 1);
        assert!(MediaTime::indefinite().convert_scale(10, RoundingMethod::TowardZero).is_indefinite());
    }

    #[test]
    fn a_time_range_covers_its_start_up_to_its_end() {
        let range = TimeRange::new(MediaTime::new(1, 1), MediaTime::new(2, 1));
        assert_eq!(range.end().seconds(), 3.0);
        assert!(range.contains_time(&MediaTime::new(1, 1)));
        assert!(range.contains_time(&MediaTime::new(2, 1)));
        assert!(!range.contains_time(&MediaTime::new(3, 1)));
        assert!(!range.contains_time(&MediaTime::new(0, 1)));
        assert!(TimeRange::zero().is_empty());
        assert!(!TimeRange::invalid().is_valid());
        let backwards = TimeRange::from_start_end(MediaTime::new(5, 1), MediaTime::new(2, 1));
        assert!(!backwards.contains_time(&MediaTime::new(3, 1)));
    }
}
