//! Magnetic field readings and compass headings: a tilt-compensated magnetic heading from the field and the gravity
//! direction, a true heading from a declination the caller supplies, and a hard-iron calibration that learns the bias
//! the device's own parts put on the field reading.
//!
//! Frame: the device frame of [`crate::motion::Vec3`] (x to the right of the screen, y up the screen, z out of it).
//! Gravity is the direction the device reads as down, as [`crate::motion::DeviceMotion::gravity`] gives it. The field
//! is in microtesla. A heading is in degrees, clockwise from north, of the top edge of the device seen from above.
//! The module has no sensor, no clock and no declination table: the caller reads the field and supplies the
//! declination.
use crate::motion::Vec3;

/// A calibration trusts its offset only once every axis has swept at least this many microtesla.
const MIN_SPAN_UT: f64 = 20.0;

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// The magnetic heading of the device's top edge, in degrees clockwise from magnetic north, in `0..360`.
///
/// `field_ut` is the field in the device frame; `gravity` is the direction of down in the device frame. The field is
/// tilted back to the horizontal plane with the gravity direction, so the heading holds whatever the device's tilt.
/// Returns `None` when a vector is zero or not finite, when the field is parallel to gravity, or when the device
/// points straight up or down (its top edge then has no heading).
pub fn magnetic_heading(field_ut: Vec3, gravity: Vec3) -> Option<f64> {
    if !finite(field_ut) || !finite(gravity) {
        return None;
    }
    let g = gravity.norm();
    if g < 1e-9 {
        return None;
    }
    let up = Vec3::new(-gravity.x / g, -gravity.y / g, -gravity.z / g);
    let horizontal = |v: Vec3| {
        let along = dot(v, up);
        Vec3::new(v.x - along * up.x, v.y - along * up.y, v.z - along * up.z)
    };
    let north = horizontal(field_ut);
    let top = horizontal(Vec3::new(0.0, 1.0, 0.0));
    if north.norm() < 1e-9 || top.norm() < 1e-9 {
        return None;
    }
    // the turn from the top edge to north, counter-clockwise about up, is the heading of the top edge clockwise
    // from north
    let turn = dot(cross(top, north), up).atan2(dot(top, north));
    Some(turn.to_degrees().rem_euclid(360.0))
}

/// The true heading from a magnetic one. `declination_deg` is the angle from true north to magnetic north, positive
/// when magnetic north lies east of true north. The caller takes it from a model or a table; this module has none.
pub fn true_heading(magnetic_deg: f64, declination_deg: f64) -> f64 {
    (magnetic_deg + declination_deg).rem_euclid(360.0)
}

/// Learns the hard-iron offset. It keeps the span of each axis over the readings it has seen; once every axis has
/// swept at least [`MIN_SPAN_UT`], the offset is the centre of that box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Calibration {
    low: Vec3,
    high: Vec3,
    count: u32,
}

impl Calibration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a field reading in microtesla. Returns `false`, and keeps the state, for a reading that is not finite.
    pub fn add(&mut self, field_ut: Vec3) -> bool {
        if !finite(field_ut) {
            return false;
        }
        if self.count == 0 {
            self.low = field_ut;
            self.high = field_ut;
        } else {
            self.low = Vec3::new(self.low.x.min(field_ut.x), self.low.y.min(field_ut.y), self.low.z.min(field_ut.z));
            self.high = Vec3::new(self.high.x.max(field_ut.x), self.high.y.max(field_ut.y), self.high.z.max(field_ut.z));
        }
        self.count = self.count.saturating_add(1);
        true
    }

    /// Whether every axis has swept far enough to trust the offset.
    pub fn is_calibrated(&self) -> bool {
        self.count > 0
            && self.high.x - self.low.x >= MIN_SPAN_UT
            && self.high.y - self.low.y >= MIN_SPAN_UT
            && self.high.z - self.low.z >= MIN_SPAN_UT
    }

    /// The bias to subtract from raw readings, once calibrated.
    pub fn offset(&self) -> Option<Vec3> {
        if !self.is_calibrated() {
            return None;
        }
        Some(Vec3::new(
            (self.low.x + self.high.x) / 2.0,
            (self.low.y + self.high.y) / 2.0,
            (self.low.z + self.high.z) / 2.0,
        ))
    }

    /// A raw reading with the offset removed, once calibrated.
    pub fn correct(&self, field_ut: Vec3) -> Option<Vec3> {
        let o = self.offset()?;
        Some(Vec3::new(field_ut.x - o.x, field_ut.y - o.y, field_ut.z - o.z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two headings in degrees are the same direction.
    fn same_heading(a: f64, b: f64) -> bool {
        let d = (a - b).rem_euclid(360.0);
        d < 1e-6 || 360.0 - d < 1e-6
    }

    #[test]
    fn a_flat_device_reads_the_four_cardinal_headings() {
        // gravity reads down the z axis; the field has a downward component and a horizontal one
        let down = Vec3::new(0.0, 0.0, -1.0);
        // top edge points north: north is along +y
        assert!(same_heading(magnetic_heading(Vec3::new(0.0, 20.0, -40.0), down).unwrap(), 0.0));
        // top edge points east: north lies to the left, along -x
        assert!(same_heading(magnetic_heading(Vec3::new(-20.0, 0.0, -40.0), down).unwrap(), 90.0));
        // top edge points south: north lies behind, along -y
        assert!(same_heading(magnetic_heading(Vec3::new(0.0, -20.0, -40.0), down).unwrap(), 180.0));
        // top edge points west: north lies to the right, along +x
        assert!(same_heading(magnetic_heading(Vec3::new(20.0, 0.0, -40.0), down).unwrap(), 270.0));
    }

    #[test]
    fn a_tilted_device_keeps_its_heading() {
        // the top edge tilts 30 degrees up towards the sky and still points at north
        let (s, c) = 30f64.to_radians().sin_cos();
        let gravity = Vec3::new(0.0, -s, -c);
        let field = Vec3::new(0.0, 20.0 * c - 40.0 * s, -20.0 * s - 40.0 * c);
        assert!(same_heading(magnetic_heading(field, gravity).unwrap(), 0.0));
    }

    #[test]
    fn headings_are_undefined_without_a_usable_direction() {
        let down = Vec3::new(0.0, 0.0, -1.0);
        let field = Vec3::new(0.0, 20.0, -40.0);
        // no field, no gravity, or a value that is not finite
        assert_eq!(magnetic_heading(Vec3::default(), down), None);
        assert_eq!(magnetic_heading(field, Vec3::default()), None);
        assert_eq!(magnetic_heading(Vec3::new(f64::NAN, 0.0, 0.0), down), None);
        // the device stands upright on its top edge: the top edge points at the sky and has no heading
        assert_eq!(magnetic_heading(field, Vec3::new(0.0, -1.0, 0.0)), None);
        // the field is straight along gravity: there is no horizontal component to point with
        assert_eq!(magnetic_heading(Vec3::new(0.0, 0.0, -40.0), down), None);
    }

    #[test]
    fn true_heading_applies_the_declination_and_wraps() {
        assert!(same_heading(true_heading(350.0, 15.0), 5.0));
        assert!(same_heading(true_heading(0.0, -10.0), 350.0));
        assert!(same_heading(true_heading(120.0, 0.0), 120.0));
    }

    #[test]
    fn calibration_finds_the_centre_of_a_swept_field() {
        let mut cal = Calibration::new();
        // a field of +-30 microtesla around a bias of (5, -8, 12) on each axis
        for &(x, y, z) in &[(35.0, -8.0, 12.0), (-25.0, -8.0, 12.0), (5.0, 22.0, 12.0), (5.0, -38.0, 12.0), (5.0, -8.0, 42.0), (5.0, -8.0, -18.0)] {
            assert!(cal.add(Vec3::new(x, y, z)));
        }
        assert!(cal.is_calibrated());
        let offset = cal.offset().unwrap();
        assert!((offset.x - 5.0).abs() < 1e-9 && (offset.y + 8.0).abs() < 1e-9 && (offset.z - 12.0).abs() < 1e-9);
        let corrected = cal.correct(Vec3::new(5.0, -8.0, 12.0)).unwrap();
        assert!(corrected.norm() < 1e-9);
    }

    #[test]
    fn calibration_waits_for_every_axis_and_refuses_bad_readings() {
        let mut cal = Calibration::new();
        assert!(!cal.add(Vec3::new(f64::NAN, 0.0, 0.0)), "a reading that is not finite is refused");
        assert!(!cal.is_calibrated());
        assert_eq!(cal.offset(), None);
        // a sweep on x and y only leaves z narrow: still not calibrated
        cal.add(Vec3::new(-30.0, -30.0, 0.0));
        cal.add(Vec3::new(30.0, 30.0, 1.0));
        assert!(!cal.is_calibrated());
        assert_eq!(cal.correct(Vec3::new(1.0, 1.0, 1.0)), None);
        cal.add(Vec3::new(0.0, 0.0, 40.0));
        assert!(cal.is_calibrated());
    }
}
