//! Motion profile of a controller: gravity, user acceleration, rotation rate and attitude, computed from raw
//! accelerometer samples (in g) and gyroscope samples (in rad/s). Acceleration is split by a first-order low-pass
//! filter: the filtered part is gravity, which moves slowly, and the rest is the acceleration the user applies.
//! Attitude is the roll and pitch of the gravity vector. Yaw cannot be read from gravity, so it stays at zero until
//! a fusion with the rotation rate is added.
//!
//! Pure: no sensor device and no thread. The event loop reads the raw samples and calls `update`.

use std::f64::consts::PI;

/// A three-component vector: an acceleration in g, or a rotation rate in rad/s.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }

    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    fn scale(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }

    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

/// The attitude as roll and pitch in radians, from the gravity direction. Roll is the rotation about the x axis
/// (atan2 of y over z); pitch is the rotation about the y axis (atan2 of -x over the length of y and z).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EulerAngles {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
}

/// The motion state of one controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionProfile {
    alpha: f64,
    seeded: bool,
    acceleration: Vec3,
    gravity: Vec3,
    rotation_rate: Vec3,
}

impl MotionProfile {
    /// A profile whose gravity filter moves `alpha` of the way to each new sample. `alpha` is clamped to 0..=1:
    /// 1 makes all acceleration gravity, 0 freezes gravity at its first sample.
    pub fn new(alpha: f64) -> Self {
        MotionProfile {
            alpha: alpha.clamp(0.0, 1.0),
            seeded: false,
            acceleration: Vec3::default(),
            gravity: Vec3::default(),
            rotation_rate: Vec3::default(),
        }
    }

    /// Feeds one pair of samples. The first acceleration sample sets gravity directly; later ones move it by
    /// `alpha` of the difference.
    pub fn update(&mut self, acceleration: Vec3, rotation_rate: Vec3) {
        if self.seeded {
            self.gravity = self.gravity.add(acceleration.sub(self.gravity).scale(self.alpha));
        } else {
            self.gravity = acceleration;
            self.seeded = true;
        }
        self.acceleration = acceleration;
        self.rotation_rate = rotation_rate;
    }

    /// The total acceleration, gravity included.
    pub fn acceleration(&self) -> Vec3 {
        self.acceleration
    }

    /// The gravity part of the acceleration.
    pub fn gravity(&self) -> Vec3 {
        self.gravity
    }

    /// The acceleration the user applies: total acceleration minus gravity.
    pub fn user_acceleration(&self) -> Vec3 {
        self.acceleration.sub(self.gravity)
    }

    /// The last rotation rate sample.
    pub fn rotation_rate(&self) -> Vec3 {
        self.rotation_rate
    }

    /// The attitude derived from the gravity vector. Zero until the first update.
    pub fn attitude(&self) -> EulerAngles {
        let g = self.gravity;
        let roll = g.y.atan2(g.z);
        let pitch = (-g.x).atan2((g.y * g.y + g.z * g.z).sqrt());
        EulerAngles { roll, pitch, yaw: 0.0 }
    }
}

/// Converts an angle in radians to degrees.
pub fn to_degrees(radians: f64) -> f64 {
    radians * 180.0 / PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn near3(v: Vec3, x: f64, y: f64, z: f64) -> bool {
        near(v.x, x) && near(v.y, y) && near(v.z, z)
    }

    #[test]
    fn first_sample_seeds_gravity_and_leaves_no_user_acceleration() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        assert!(near3(m.gravity(), 0.0, 0.0, 1.0));
        assert!(near3(m.user_acceleration(), 0.0, 0.0, 0.0));
    }

    #[test]
    fn filter_moves_gravity_half_way_and_the_rest_is_user_acceleration() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        // Gravity: (0,0,1) + 0.5 * ((0,0,3) - (0,0,1)) = (0,0,2). User: (0,0,3) - (0,0,2) = (0,0,1).
        m.update(Vec3::new(0.0, 0.0, 3.0), Vec3::default());
        assert!(near3(m.gravity(), 0.0, 0.0, 2.0));
        assert!(near3(m.user_acceleration(), 0.0, 0.0, 1.0));
        assert!(near3(m.acceleration(), 0.0, 0.0, 3.0));
    }

    #[test]
    fn alpha_is_clamped_to_one() {
        let mut m = MotionProfile::new(2.0);
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        // With alpha 1, gravity jumps to the new sample and no user acceleration remains.
        m.update(Vec3::new(0.0, 0.0, 5.0), Vec3::default());
        assert!(near3(m.gravity(), 0.0, 0.0, 5.0));
        assert!(near3(m.user_acceleration(), 0.0, 0.0, 0.0));
    }

    #[test]
    fn level_controller_has_zero_roll_and_pitch() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        let a = m.attitude();
        assert!(near(a.roll, 0.0) && near(a.pitch, 0.0) && near(a.yaw, 0.0));
    }

    #[test]
    fn gravity_on_y_axis_is_a_quarter_turn_of_roll() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(0.0, 1.0, 0.0), Vec3::default());
        let a = m.attitude();
        // roll = atan2(1, 0) = pi/2; pitch = atan2(-0, 1) = 0.
        assert!(near(a.roll, FRAC_PI_2));
        assert!(near(a.pitch, 0.0));
        assert!(near(to_degrees(a.roll), 90.0));
    }

    #[test]
    fn gravity_on_negative_x_axis_is_a_quarter_turn_of_pitch() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(-1.0, 0.0, 0.0), Vec3::default());
        // pitch = atan2(1, 0) = pi/2; roll = atan2(0, 0) = 0.
        let a = m.attitude();
        assert!(near(a.pitch, FRAC_PI_2));
        assert!(near(a.roll, 0.0));
    }

    #[test]
    fn rotation_rate_is_passed_through_from_the_last_sample() {
        let mut m = MotionProfile::new(0.5);
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.1, 0.2, 0.3));
        m.update(Vec3::new(0.0, 0.0, 1.0), Vec3::new(-0.5, 0.0, 2.0));
        assert!(near3(m.rotation_rate(), -0.5, 0.0, 2.0));
    }
}
