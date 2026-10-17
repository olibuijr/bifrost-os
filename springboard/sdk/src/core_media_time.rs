//! CoreMedia time for apps: `CMTime` (a rational value over a timescale, with invalid, indefinite and infinite states)
//! and `CMTimeRange`. The model lives in `bifrost_ui::media_time` and the media pipeline uses the same type, so a
//! time an app builds and a time the player reports compare, convert and round the same way.
//!
//! ```ignore
//! let half = MediaTime::new(1, 2);
//! assert_eq!(half, MediaTime::new(2, 4)); // equal across timescales
//! assert_eq!(MediaTime::new(3, 2).seconds(), 1.5);
//! ```

pub use bifrost_ui::media_time::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn a_time_is_its_value_over_its_timescale() {
        let t = MediaTime::new(3, 2);
        assert!(t.is_valid() && t.is_numeric());
        assert_eq!(t.value(), 3);
        assert_eq!(t.timescale(), 2);
        assert_eq!(t.seconds(), 1.5);
    }

    #[test]
    fn a_timescale_that_is_not_positive_is_invalid() {
        assert!(!MediaTime::new(1, 0).is_valid());
        assert!(!MediaTime::new(1, -600).is_valid());
        assert!(MediaTime::from_seconds(f64::NAN, 600).seconds().is_nan());
    }

    #[test]
    fn times_on_different_timescales_are_equal_when_they_name_the_same_instant() {
        assert_eq!(MediaTime::new(1, 2), MediaTime::new(2, 4));
        assert_eq!(MediaTime::new(1500, 1000).compare(&MediaTime::new(3, 2)), Some(Ordering::Equal));
        assert_eq!(MediaTime::new(1, 3).compare(&MediaTime::new(1, 2)), Some(Ordering::Less));
    }

    #[test]
    fn an_invalid_or_indefinite_time_equals_nothing_not_even_itself() {
        assert_ne!(MediaTime::invalid(), MediaTime::invalid());
        assert_ne!(MediaTime::indefinite(), MediaTime::indefinite());
    }

    #[test]
    fn from_seconds_rounds_to_the_preferred_timescale_and_flags_it() {
        let exact = MediaTime::from_seconds(0.5, 600);
        assert_eq!((exact.value(), exact.timescale()), (300, 600));
        assert!(!exact.has_been_rounded());
        let rounded = MediaTime::from_seconds(1.0 / 3.0, 10);
        assert_eq!(rounded.value(), 3);
        assert!(rounded.has_been_rounded());
    }

    #[test]
    fn arithmetic_keeps_the_exact_rational_value() {
        let sum = MediaTime::new(1, 3) + MediaTime::new(1, 6);
        assert_eq!(sum, MediaTime::new(1, 2));
        let diff = MediaTime::new(1, 2) - MediaTime::new(1, 4);
        assert_eq!(diff, MediaTime::new(1, 4));
    }

    #[test]
    fn convert_scale_applies_the_rounding_method() {
        let t = MediaTime::new(1, 3);
        assert_eq!(t.convert_scale(10, RoundingMethod::TowardZero).value(), 3);
        assert_eq!(t.convert_scale(10, RoundingMethod::TowardPositiveInfinity).value(), 4);
    }

    #[test]
    fn a_time_range_contains_its_start_and_not_its_end() {
        let range = TimeRange::new(MediaTime::new(1, 1), MediaTime::new(2, 1));
        assert_eq!(range.end(), MediaTime::new(3, 1));
        assert!(range.contains_time(&MediaTime::new(1, 1)));
        assert!(range.contains_time(&MediaTime::new(5, 2)));
        assert!(!range.contains_time(&MediaTime::new(3, 1)));
    }
}
