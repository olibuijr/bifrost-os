//! Digitizer stylus input as a pure model. A pen digitizer reports raw values in its own logical ranges: contact
//! pressure, two tilt angles, the pen's position and which end is in use. This module maps those values onto the
//! `StrokePoint` fields the drawing model stores (force in 0..=1, altitude and azimuth in radians). No device is
//! opened here; the input driver fills in a `StylusReport` and asks for the point.
use crate::drawing::StrokePoint;
use std::f32::consts::FRAC_PI_2;

/// The logical range a device reports one field in, inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalRange {
    pub min: f32,
    pub max: f32,
}

impl LogicalRange {
    /// Maps `v` linearly onto 0..=1 across the range and clamps values outside it. A range whose max is not above its
    /// min maps every value to 0.
    pub fn unit(self, v: f32) -> f32 {
        if self.max <= self.min {
            return 0.0;
        }
        ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    /// Maps `v` linearly onto -1..=1, with the midpoint of the range at 0. A degenerate range maps to 0.
    pub fn signed(self, v: f32) -> f32 {
        if self.max <= self.min {
            return 0.0;
        }
        self.unit(v) * 2.0 - 1.0
    }
}

/// The ranges a digitizer reports its pressure and tilt in. Tilt is one range for both axes, and its ends are the
/// pen lying flat (plus or minus a right angle from upright).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StylusProfile {
    pub pressure: LogicalRange,
    pub tilt: LogicalRange,
}

/// One raw report from a pen. `x` and `y` are already in drawing space; the driver maps them from the device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StylusReport {
    pub x: f32,
    pub y: f32,
    /// Seconds since the stroke began.
    pub time_offset: f32,
    /// Raw pressure, in the profile's pressure range.
    pub pressure: f32,
    /// Raw tilt about the x and y axes, in the profile's tilt range.
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// True while the tip touches the surface. Hover reports are not stroke points.
    pub touching: bool,
    /// True while the eraser end is in use.
    pub eraser: bool,
}

/// The altitude (radians above the surface) and azimuth (radians, the direction the pen leans towards) of a pen whose
/// tilt is `tilt_x` and `tilt_y` radians about the two axes. The lean is the length of the vector of the two sines,
/// so an upright pen gives altitude pi/2 and a pen lying flat gives altitude 0.
pub fn pen_angles(tilt_x: f32, tilt_y: f32) -> (f32, f32) {
    let (sx, sy) = (tilt_x.sin(), tilt_y.sin());
    let lean = (sx * sx + sy * sy).sqrt().min(1.0).asin();
    (FRAC_PI_2 - lean, sy.atan2(sx))
}

impl StylusReport {
    /// The stroke point for this report, or `None` when the tip is not touching. The force is the pressure mapped
    /// onto 0..=1. The tilt values are mapped onto -pi/2..=pi/2 radians before the pen angles are worked out.
    pub fn to_point(&self, profile: &StylusProfile) -> Option<StrokePoint> {
        if !self.touching {
            return None;
        }
        let tilt_x = profile.tilt.signed(self.tilt_x) * FRAC_PI_2;
        let tilt_y = profile.tilt.signed(self.tilt_y) * FRAC_PI_2;
        let (altitude, azimuth) = pen_angles(tilt_x, tilt_y);
        Some(StrokePoint {
            x: self.x,
            y: self.y,
            time_offset: self.time_offset,
            size: (0.0, 0.0),
            opacity: 1.0,
            force: profile.pressure.unit(self.pressure),
            azimuth,
            altitude,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    const PROFILE: StylusProfile = StylusProfile {
        pressure: LogicalRange { min: 0.0, max: 1024.0 },
        tilt: LogicalRange { min: -90.0, max: 90.0 },
    };

    fn report(pressure: f32, tilt_x: f32, tilt_y: f32) -> StylusReport {
        StylusReport { x: 3.0, y: 4.0, time_offset: 0.5, pressure, tilt_x, tilt_y, touching: true, eraser: false }
    }

    #[test]
    fn an_upright_pen_has_full_altitude_and_no_azimuth() {
        let (alt, az) = pen_angles(0.0, 0.0);
        assert!(near(alt, FRAC_PI_2));
        assert!(near(az, 0.0));
    }

    #[test]
    fn a_pen_lying_flat_has_no_altitude() {
        // Tilt of a right angle about x: the pen lies along the surface, pointing along x.
        let (alt, az) = pen_angles(FRAC_PI_2, 0.0);
        assert!(near(alt, 0.0));
        assert!(near(az, 0.0));
        // The same tilt about y points the azimuth along y.
        let (alt_y, az_y) = pen_angles(0.0, FRAC_PI_2);
        assert!(near(alt_y, 0.0));
        assert!(near(az_y, FRAC_PI_2));
    }

    #[test]
    fn logical_ranges_map_to_unit_and_signed_values() {
        let r = LogicalRange { min: 0.0, max: 4096.0 };
        assert!(near(r.unit(1024.0), 0.25));
        assert!(near(r.unit(5000.0), 1.0), "values above the range clamp to 1");
        assert!(near(r.unit(-5.0), 0.0), "values below the range clamp to 0");
        assert!(near(LogicalRange { min: 0.0, max: 100.0 }.signed(50.0), 0.0));
        assert!(near(LogicalRange { min: 0.0, max: 100.0 }.signed(0.0), -1.0));
        assert!(near(LogicalRange { min: 5.0, max: 5.0 }.unit(9.0), 0.0), "a degenerate range maps to 0");
    }

    #[test]
    fn a_touching_report_becomes_a_stroke_point_with_mapped_force_and_angles() {
        // Half pressure, upright tilt (0 on a -90..90 range is the middle).
        let p = report(512.0, 0.0, 0.0).to_point(&PROFILE).expect("touching");
        assert_eq!((p.x, p.y, p.time_offset), (3.0, 4.0, 0.5));
        assert!(near(p.force, 0.5));
        assert!(near(p.altitude, FRAC_PI_2));
        assert!(near(p.azimuth, 0.0));
        assert_eq!(p.opacity, 1.0);
    }

    #[test]
    fn full_tilt_on_the_device_range_is_a_right_angle() {
        // A raw tilt of 90 is the far end of the range, so the tilt is pi/2 and the pen lies flat along x.
        let p = report(0.0, 90.0, 0.0).to_point(&PROFILE).expect("touching");
        assert!(near(p.altitude, 0.0));
        assert!(near(p.azimuth, 0.0));
        assert_eq!(p.force, 0.0);
    }

    #[test]
    fn hover_reports_are_not_stroke_points() {
        let mut hover = report(800.0, 0.0, 0.0);
        hover.touching = false;
        assert_eq!(hover.to_point(&PROFILE), None);
        assert!(!report(0.0, 0.0, 0.0).eraser);
        let mut erase = report(0.0, 0.0, 0.0);
        erase.eraser = true;
        assert!(erase.eraser, "the eraser flag is carried on the report");
    }
}
