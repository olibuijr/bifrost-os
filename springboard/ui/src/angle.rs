//! A typed angle for app code (`Angle`, the value of `Angle.degrees(_)` and `Angle.radians(_)`). The angle is stored in
//! radians, the unit the trigonometry in `f32::sin` and `f32::cos` takes, and reads back in either unit. Arithmetic
//! keeps the unit: adding two angles adds their radians, scaling an angle multiplies it.

use std::f32::consts::{PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Angle {
    radians: f32,
}

impl Angle {
    pub const ZERO: Angle = Angle { radians: 0.0 };

    pub fn radians(radians: f32) -> Angle {
        Angle { radians }
    }

    pub fn degrees(degrees: f32) -> Angle {
        Angle { radians: degrees * PI / 180.0 }
    }

    pub fn as_radians(self) -> f32 {
        self.radians
    }

    pub fn as_degrees(self) -> f32 {
        self.radians * 180.0 / PI
    }

    pub fn sin(self) -> f32 {
        self.radians.sin()
    }

    pub fn cos(self) -> f32 {
        self.radians.cos()
    }

    pub fn tan(self) -> f32 {
        self.radians.tan()
    }

    /// The same direction in the half-open range (-180, 180] degrees.
    pub fn normalized(self) -> Angle {
        let r = self.radians.rem_euclid(TAU);
        Angle { radians: if r > PI { r - TAU } else { r } }
    }
}

impl Add for Angle {
    type Output = Angle;
    fn add(self, rhs: Angle) -> Angle {
        Angle { radians: self.radians + rhs.radians }
    }
}

impl Sub for Angle {
    type Output = Angle;
    fn sub(self, rhs: Angle) -> Angle {
        Angle { radians: self.radians - rhs.radians }
    }
}

impl Neg for Angle {
    type Output = Angle;
    fn neg(self) -> Angle {
        Angle { radians: -self.radians }
    }
}

impl Mul<f32> for Angle {
    type Output = Angle;
    fn mul(self, factor: f32) -> Angle {
        Angle { radians: self.radians * factor }
    }
}
