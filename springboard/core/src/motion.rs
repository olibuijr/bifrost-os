//! Device motion from raw inertial samples: a gravity estimate, the acceleration the user gives the device,
//! the rotation rate and an attitude (roll, pitch, yaw and a unit quaternion).
//!
//! The attitude is a complementary filter. The gyroscope is integrated for the fast changes, and the
//! gravity direction pulls roll and pitch back, so they cannot drift. Yaw comes from the gyroscope alone,
//! in an arbitrary reference frame: there is no magnetometer in the input.
//!
//! Units: readings in g, as the sensors report them (a device at rest reads one g along the axis that
//! points up off the ground); rotation rates in radians per second; times in seconds on any monotonic
//! clock. Angles are radians. Roll is zero with the screen flat face up and a quarter turn when the device
//! stands upright in portrait; pitch is zero flat. The estimator owns no thread and no clock: the caller
//! feeds it samples at its own cadence.
use std::f64::consts::{PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

/// How long the slow gravity estimate takes to follow a change, in seconds.
const GRAVITY_TIME_CONSTANT: f64 = 1.0;

/// A vector in the device frame: x to the right of the screen, y up the screen, z out of the screen.
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

    pub fn norm(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// The device's orientation: roll about x, pitch about y, yaw about z, and the same as a unit quaternion
/// `[w, x, y, z]`, applied as yaw, then pitch, then roll.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attitude {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
    pub quaternion: [f64; 4],
}

impl Attitude {
    pub fn from_angles(roll: f64, pitch: f64, yaw: f64) -> Attitude {
        let (sr, cr) = (roll / 2.0).sin_cos();
        let (sp, cp) = (pitch / 2.0).sin_cos();
        let (sy, cy) = (yaw / 2.0).sin_cos();
        let quaternion = [
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        ];
        Attitude { roll, pitch, yaw, quaternion }
    }
}

/// One estimate: what the device is doing at `timestamp`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceMotion {
    pub timestamp: f64,
    pub attitude: Attitude,
    /// the gyroscope reading, in radians per second
    pub rotation_rate: Vec3,
    /// the direction of gravity in the device frame, in g (it points down, so a flat device reads (0, 0, -1))
    pub gravity: Vec3,
    /// the reading without gravity: the acceleration the user gives the device, in g
    pub user_acceleration: Vec3,
}

/// Turns accelerometer and gyroscope samples into [`DeviceMotion`] estimates.
#[derive(Clone, Copy, Debug)]
pub struct MotionEstimator {
    /// the share of the gyroscope's angle kept each sample; the rest comes from gravity
    blend: f64,
    last: Option<DeviceMotion>,
}

impl MotionEstimator {
    /// `blend` is clamped to 0..=1. Values near 1 trust the gyroscope; 0.98 suits samples at about 100 Hz.
    pub fn new(blend: f64) -> Self {
        let blend = if blend.is_finite() { blend.clamp(0.0, 1.0) } else { 0.98 };
        MotionEstimator { blend, last: None }
    }

    /// The newest estimate, if any sample has been accepted.
    pub fn latest(&self) -> Option<DeviceMotion> {
        self.last
    }

    /// Forgets the attitude and gravity: the next sample starts a new estimate.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Feeds one accelerometer `reading` (g) and gyroscope `rate` (rad/s) taken at time `t` (s).
    /// Returns the new estimate, or `None` when a value is not finite; the estimator then keeps its state.
    /// A sample with the same or an earlier time integrates nothing and only corrects the tilt.
    pub fn update(&mut self, t: f64, reading: Vec3, rate: Vec3) -> Option<DeviceMotion> {
        if !t.is_finite() || !reading.is_finite() || !rate.is_finite() {
            return None;
        }
        let next = match self.last {
            // the first sample assumes the device is at rest, so the reading is gravity
            None => {
                let (roll, pitch) = tilt(reading);
                DeviceMotion {
                    timestamp: t,
                    attitude: Attitude::from_angles(roll, pitch, 0.0),
                    rotation_rate: rate,
                    gravity: -reading,
                    user_acceleration: Vec3::default(),
                }
            }
            Some(prev) => {
                let dt = (t - prev.timestamp).max(0.0);
                // the gravity estimate is the reading low-passed with a time constant of one second
                let alpha = dt / (GRAVITY_TIME_CONSTANT + dt);
                let up_prev = -prev.gravity;
                let up = up_prev + (reading - up_prev) * alpha;
                let (roll_tilt, pitch_tilt) = tilt(up);
                let roll = self.blend * (prev.attitude.roll + rate.x * dt) + (1.0 - self.blend) * roll_tilt;
                let pitch = self.blend * (prev.attitude.pitch + rate.y * dt) + (1.0 - self.blend) * pitch_tilt;
                let yaw = wrap(prev.attitude.yaw + rate.z * dt);
                DeviceMotion {
                    timestamp: t,
                    attitude: Attitude::from_angles(roll, pitch, yaw),
                    rotation_rate: rate,
                    gravity: -up,
                    user_acceleration: reading - up,
                }
            }
        };
        self.last = Some(next);
        Some(next)
    }
}

/// Roll and pitch of the device from the direction of up in the device frame. Zero when the screen lies flat.
fn tilt(up: Vec3) -> (f64, f64) {
    (up.y.atan2(up.z), (-up.x).atan2(up.y.hypot(up.z)))
}

/// Wraps an angle into (-π, π].
fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_device_at_rest_reads_no_tilt_and_no_user_acceleration() {
        let mut m = MotionEstimator::new(0.98);
        let mut last = None;
        for i in 0..20 {
            last = m.update(i as f64 * 0.1, Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        }
        let d = last.unwrap();
        assert!(close(d.attitude.roll, 0.0) && close(d.attitude.pitch, 0.0) && close(d.attitude.yaw, 0.0));
        assert!(close(d.gravity.z, -1.0) && close(d.gravity.x, 0.0));
        assert!(d.user_acceleration.norm() < 1e-9);
        assert_eq!(m.latest(), last);
    }

    #[test]
    fn an_upright_device_reports_a_quarter_turn_of_roll() {
        // the top edge points up in portrait: the reading is one g along y
        let mut m = MotionEstimator::new(0.98);
        let d = m.update(0.0, Vec3::new(0.0, 1.0, 0.0), Vec3::default()).unwrap();
        assert!(close(d.attitude.roll, std::f64::consts::FRAC_PI_2), "roll {}", d.attitude.roll);
        assert!(close(d.attitude.pitch, 0.0));
    }

    #[test]
    fn the_gyroscope_integrates_yaw_and_reports_the_rate() {
        let mut m = MotionEstimator::new(0.98);
        let rate = Vec3::new(0.0, 0.0, 1.0);
        let mut last = None;
        // 51 samples 10 ms apart: the first only starts the estimate, then 50 steps add 0.5 rad
        for i in 0..=50 {
            last = m.update(i as f64 * 0.01, Vec3::new(0.0, 0.0, 1.0), rate);
        }
        let d = last.unwrap();
        assert!(close(d.attitude.yaw, 0.5), "yaw {}", d.attitude.yaw);
        assert_eq!(d.rotation_rate, rate);
    }

    #[test]
    fn gravity_stops_a_constant_gyroscope_bias_from_drifting_the_roll() {
        // a 0.1 rad/s bias on x with the screen flat: the complementary filter settles near a*b*dt/(1-a)
        let mut m = MotionEstimator::new(0.98);
        let mut last = None;
        for i in 0..1000 {
            last = m.update(i as f64 * 0.01, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.1, 0.0, 0.0));
        }
        let roll = last.unwrap().attitude.roll;
        assert!(roll > 0.04 && roll < 0.06, "roll {roll}");
    }

    #[test]
    fn user_acceleration_is_the_reading_minus_the_slow_gravity_estimate() {
        let mut m = MotionEstimator::new(0.98);
        m.update(0.0, Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        // a 0.2 g push for 0.1 s: the gravity estimate moves by alpha = 0.1 / 1.1 of the difference
        let d = m.update(0.1, Vec3::new(0.0, 0.0, 1.2), Vec3::default()).unwrap();
        assert!(close(d.user_acceleration.z, 0.2 / 1.1), "user {}", d.user_acceleration.z);
        assert!(close(d.gravity.z, -(1.0 + 0.2 / 11.0)), "gravity {}", d.gravity.z);
    }

    #[test]
    fn non_finite_samples_are_refused_and_keep_the_state() {
        let mut m = MotionEstimator::new(0.98);
        let good = m.update(0.0, Vec3::new(0.0, 0.0, 1.0), Vec3::default()).unwrap();
        assert!(m.update(0.1, Vec3::new(f64::NAN, 0.0, 1.0), Vec3::default()).is_none());
        assert!(m.update(f64::INFINITY, Vec3::new(0.0, 0.0, 1.0), Vec3::default()).is_none());
        assert!(m.update(0.1, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, f64::NAN, 0.0)).is_none());
        assert_eq!(m.latest(), Some(good));
        m.reset();
        assert_eq!(m.latest(), None);
    }

    #[test]
    fn the_quaternion_is_a_unit_quaternion_and_a_quarter_turn_about_x() {
        let q = Attitude::from_angles(std::f64::consts::FRAC_PI_2, 0.0, 0.0).quaternion;
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert!(close(q[0], h) && close(q[1], h) && close(q[2], 0.0) && close(q[3], 0.0), "{q:?}");
        for (r, p, y) in [(0.3, -1.1, 2.5), (-2.0, 0.7, -0.4), (3.0, 1.2, 0.0)] {
            let q = Attitude::from_angles(r, p, y).quaternion;
            let n: f64 = q.iter().map(|v| v * v).sum();
            assert!(close(n, 1.0), "norm {n}");
        }
    }

    #[test]
    fn yaw_wraps_into_the_half_open_range() {
        let mut m = MotionEstimator::new(0.98);
        m.update(0.0, Vec3::new(0.0, 0.0, 1.0), Vec3::default());
        // 4 rad of yaw in 1 s: the reported yaw stays inside (-pi, pi]
        let d = m.update(1.0, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 4.0)).unwrap();
        assert!(close(d.attitude.yaw, 4.0 - TAU), "yaw {}", d.attitude.yaw);
    }
}
