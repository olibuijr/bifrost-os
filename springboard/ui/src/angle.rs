//! A typed angle for app code (`Angle`, the value of `Angle.degrees(_)` and `Angle.radians(_)`). The angle is stored in
//! radians, the unit the trigonometry in `f32::sin` and `f32::cos` takes, and reads back in either unit. Arithmetic
//! keeps the unit: adding two angles adds their radians, scaling an angle multiplies it.

use std::f32::consts::{PI, TAU};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

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

    /// The sine and cosine together, for code that needs both of a direction.
    pub fn sin_cos(self) -> (f32, f32) {
        self.radians.sin_cos()
    }

    /// The angle of the vector `(x, y)` from the positive x axis, in [-180, 180] degrees.
    pub fn atan2(y: f32, x: f32) -> Angle {
        Angle { radians: y.atan2(x) }
    }

    /// The angle whose sine is `value`, in [-90, 90] degrees. None when `value` is outside [-1, 1].
    pub fn asin(value: f32) -> Option<Angle> {
        (-1.0..=1.0).contains(&value).then(|| Angle { radians: value.asin() })
    }

    /// The angle whose cosine is `value`, in [0, 180] degrees. None when `value` is outside [-1, 1].
    pub fn acos(value: f32) -> Option<Angle> {
        (-1.0..=1.0).contains(&value).then(|| Angle { radians: value.acos() })
    }

    /// The angle `fraction` of the way from `self` to `to`. The fraction is not clamped, so values outside [0, 1]
    /// extrapolate along the same line.
    pub fn lerp(self, to: Angle, fraction: f32) -> Angle {
        Angle { radians: self.radians + (to.radians - self.radians) * fraction }
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

impl AddAssign for Angle {
    fn add_assign(&mut self, rhs: Angle) {
        self.radians += rhs.radians;
    }
}

impl SubAssign for Angle {
    fn sub_assign(&mut self, rhs: Angle) {
        self.radians -= rhs.radians;
    }
}

impl Mul<f32> for Angle {
    type Output = Angle;
    fn mul(self, factor: f32) -> Angle {
        Angle { radians: self.radians * factor }
    }
}

impl Div<f32> for Angle {
    type Output = Angle;
    fn div(self, divisor: f32) -> Angle {
        Angle { radians: self.radians / divisor }
    }
}
