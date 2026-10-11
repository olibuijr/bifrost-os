//! Device location values: a position fix with accuracy, course and speed, the motion between two fixes, and a
//! compass heading with its true or magnetic reference.
//!
//! The fix mirrors the value the official location framework delivers with each update (coordinate, altitude,
//! horizontal and vertical accuracy, course, speed and timestamp), and [`Heading`] mirrors its orientation value
//! (magnetic and true heading with an accuracy). This module only holds and derives values. It reads no sensor and
//! starts no service; a caller with a position source builds the fixes.
//!
//! Unknown values are negative, as the framework reports them: an accuracy, course or speed below zero means the
//! value is not known. Times are seconds on the caller's clock, the same clock the other UI models use.

use crate::geo::Coordinate;

/// The sixteen compass point names, starting at north and running clockwise.
const CARDINALS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW",
];

/// One position fix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocationFix {
    pub coordinate: Coordinate,
    /// Metres above sea level.
    pub altitude_m: f64,
    /// Radius of uncertainty of the horizontal position in metres; negative when the position is not valid.
    pub horizontal_accuracy_m: f64,
    /// Uncertainty of the altitude in metres; negative when the altitude is not valid.
    pub vertical_accuracy_m: f64,
    /// Degrees clockwise from true north; negative when unknown.
    pub course_deg: f64,
    /// Metres per second; negative when unknown.
    pub speed_mps: f64,
    pub timestamp: f64,
}

impl LocationFix {
    /// A fix at `coordinate` with no altitude, accuracy, course or speed. Such a fix is not valid until an
    /// accuracy is set.
    pub fn new(coordinate: Coordinate, timestamp: f64) -> Self {
        LocationFix {
            coordinate,
            altitude_m: 0.0,
            horizontal_accuracy_m: -1.0,
            vertical_accuracy_m: -1.0,
            course_deg: -1.0,
            speed_mps: -1.0,
            timestamp,
        }
    }

    /// True when the horizontal accuracy is a finite, non-negative value and the timestamp is finite.
    pub fn is_valid(&self) -> bool {
        self.horizontal_accuracy_m.is_finite() && self.horizontal_accuracy_m >= 0.0 && self.timestamp.is_finite()
    }

    /// True when the altitude has a finite, non-negative vertical accuracy.
    pub fn has_valid_altitude(&self) -> bool {
        self.altitude_m.is_finite() && self.vertical_accuracy_m.is_finite() && self.vertical_accuracy_m >= 0.0
    }

    /// True when the course is a finite bearing in 0..360.
    pub fn has_valid_course(&self) -> bool {
        self.course_deg.is_finite() && (0.0..360.0).contains(&self.course_deg)
    }

    /// True when the speed is finite and not negative.
    pub fn has_valid_speed(&self) -> bool {
        self.speed_mps.is_finite() && self.speed_mps >= 0.0
    }

    /// Seconds between the fix and `now`. Negative when the fix is later than `now`.
    pub fn age_at(&self, now: f64) -> f64 {
        now - self.timestamp
    }

    /// Great-circle distance to `other` in metres.
    pub fn distance_m(&self, other: &LocationFix) -> f64 {
        self.coordinate.distance_m(other.coordinate)
    }
}

/// Course and average speed of travel between two fixes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Initial bearing from the first fix to the second, in degrees clockwise from true north, in 0..360.
    pub course_deg: f64,
    /// Distance between the fixes divided by the time between them, in metres per second.
    pub speed_mps: f64,
}

/// The motion from `from` to `to`, derived from their positions and timestamps.
/// `None` when `to` is not strictly later than `from`, so there is no time to divide by.
pub fn motion_between(from: &LocationFix, to: &LocationFix) -> Option<Motion> {
    let dt = to.timestamp - from.timestamp;
    if !dt.is_finite() || dt <= 0.0 {
        return None;
    }
    let distance = from.coordinate.distance_m(to.coordinate);
    Some(Motion { course_deg: from.coordinate.bearing_to(to.coordinate), speed_mps: distance / dt })
}

/// A compass reading. The magnetic heading always exists; the true heading exists once a declination is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heading {
    /// Degrees clockwise from magnetic north, in 0..360.
    pub magnetic_deg: f64,
    /// Degrees clockwise from true north, in 0..360; `None` while the declination is unknown.
    pub true_deg: Option<f64>,
    /// Expected error of the reading in degrees; negative when unknown.
    pub accuracy_deg: f64,
    pub timestamp: f64,
}

impl Heading {
    /// A magnetic reading with no true heading and no accuracy. `None` when `magnetic_deg` is not finite.
    pub fn magnetic(magnetic_deg: f64, timestamp: f64) -> Option<Self> {
        magnetic_deg.is_finite().then(|| Heading {
            magnetic_deg: normalize_degrees(magnetic_deg),
            true_deg: None,
            accuracy_deg: -1.0,
            timestamp,
        })
    }

    /// Sets the true heading from the local declination, which is positive when magnetic north lies east of true
    /// north. A non-finite declination leaves the reading without a true heading.
    pub fn with_declination(mut self, declination_deg: f64) -> Self {
        if declination_deg.is_finite() {
            self.true_deg = Some(normalize_degrees(self.magnetic_deg + declination_deg));
        }
        self
    }

    /// The heading a compass rose shows: the true heading when it is known, otherwise the magnetic one.
    pub fn preferred_deg(&self) -> f64 {
        self.true_deg.unwrap_or(self.magnetic_deg)
    }

    /// The sixteen-point name of the preferred heading, such as "N" or "WSW".
    pub fn cardinal(&self) -> &'static str {
        // Each point covers 22.5 degrees, centred on its bearing. The heading is in 0..360, so the index is 0..=16
        // and the modulo folds 16 (just short of 360) back to north.
        CARDINALS[((self.preferred_deg() + 11.25) / 22.5) as usize % 16]
    }
}

/// Wraps an angle into 0..360.
pub fn normalize_degrees(degrees: f64) -> f64 {
    degrees.rem_euclid(360.0)
}

/// The signed turn from heading `from` to heading `to`, in -180..=180. Positive turns clockwise.
pub fn heading_difference_deg(from: f64, to: f64) -> f64 {
    let d = normalize_degrees(to - from);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    fn fix_at(latitude: f64, longitude: f64, timestamp: f64) -> LocationFix {
        LocationFix::new(c(latitude, longitude), timestamp)
    }

    #[test]
    fn a_new_fix_has_no_known_accuracy_course_or_speed() {
        let f = fix_at(64.0, -21.0, 10.0);
        assert!(!f.is_valid());
        assert!(!f.has_valid_altitude());
        assert!(!f.has_valid_course());
        assert!(!f.has_valid_speed());
    }

    #[test]
    fn zero_accuracy_is_valid_and_negative_or_nan_accuracy_is_not() {
        let mut f = fix_at(64.0, -21.0, 10.0);
        f.horizontal_accuracy_m = 0.0;
        assert!(f.is_valid());
        f.horizontal_accuracy_m = -1.0;
        assert!(!f.is_valid());
        f.horizontal_accuracy_m = f64::NAN;
        assert!(!f.is_valid());
    }

    #[test]
    fn motion_between_fixes_gives_course_and_average_speed() {
        // One degree of longitude on the equator is about 111.2 km; over 100 s that is about 1112 m/s.
        let from = fix_at(0.0, 0.0, 1_000.0);
        let to = fix_at(0.0, 1.0, 1_100.0);
        let m = motion_between(&from, &to).expect("later fix gives a motion");
        assert!((m.course_deg - 90.0).abs() < 1e-6);
        assert!((m.speed_mps - 1_111.95).abs() < 0.5);
    }

    #[test]
    fn motion_needs_a_later_second_fix() {
        let a = fix_at(0.0, 0.0, 50.0);
        let same_time = fix_at(0.0, 1.0, 50.0);
        let earlier = fix_at(0.0, 1.0, 40.0);
        assert!(motion_between(&a, &same_time).is_none());
        assert!(motion_between(&a, &earlier).is_none());
    }

    #[test]
    fn age_is_measured_against_the_caller_clock() {
        let f = fix_at(64.0, -21.0, 10.0);
        assert_eq!(f.age_at(15.0), 5.0);
        assert_eq!(f.age_at(4.0), -6.0);
    }

    #[test]
    fn magnetic_heading_wraps_and_names_its_point() {
        let west = Heading::magnetic(-90.0, 0.0).expect("finite heading");
        assert_eq!(west.magnetic_deg, 270.0);
        assert_eq!(west.cardinal(), "W");
        assert_eq!(Heading::magnetic(100.0, 0.0).expect("finite").cardinal(), "E");
        assert_eq!(Heading::magnetic(12.0, 0.0).expect("finite").cardinal(), "NNE");
        assert_eq!(Heading::magnetic(359.0, 0.0).expect("finite").cardinal(), "N");
    }

    #[test]
    fn true_heading_uses_the_declination_and_takes_precedence() {
        let h = Heading::magnetic(359.0, 0.0).expect("finite").with_declination(2.0);
        assert_eq!(h.true_deg, Some(1.0));
        assert_eq!(h.preferred_deg(), 1.0);
        assert_eq!(h.cardinal(), "N");
        let unknown = Heading::magnetic(90.0, 0.0).expect("finite").with_declination(f64::NAN);
        assert_eq!(unknown.true_deg, None);
        assert_eq!(unknown.preferred_deg(), 90.0);
    }

    #[test]
    fn non_finite_headings_are_refused() {
        assert!(Heading::magnetic(f64::NAN, 0.0).is_none());
        assert!(Heading::magnetic(f64::INFINITY, 0.0).is_none());
    }

    #[test]
    fn heading_difference_takes_the_short_way_round() {
        assert_eq!(heading_difference_deg(350.0, 10.0), 20.0);
        assert_eq!(heading_difference_deg(10.0, 350.0), -20.0);
        assert_eq!(heading_difference_deg(0.0, 180.0), 180.0);
        assert_eq!(heading_difference_deg(90.0, 90.0), 0.0);
    }
}
