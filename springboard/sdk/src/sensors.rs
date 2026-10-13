//! Device sensors for apps: the magnetometer heading, the altimeter pressure conversion, the motion
//! activity classifier, the pedometer and the attitude estimate. Each is the pure model the shell and the SDK share
//! (`bifrost_core`). The model takes readings as values; the sensor reader feeds it. Desktop hardware rarely has
//! these sensors, so an app should check for data before it shows a value.
//!
//! The raw accelerometer and gyroscope feed is in [`crate::motion_manager`], and the attitude and gravity estimate
//! is [`motion::MotionEstimator`].
//!
//! ```ignore
//! use bifrost_app::sensors::{magnetometer, Vec3};
//! let heading = magnetometer::magnetic_heading(Vec3::new(20.0, 0.0, -40.0), Vec3::new(0.0, 0.0, -9.8));
//! ```

pub use bifrost_core::{altitude, magnetometer, motion, motion_activity, pedometer};
pub use bifrost_core::motion::{Attitude, DeviceMotion, MotionEstimator, Vec3};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_heading_wraps_into_a_full_turn() {
        assert_eq!(magnetometer::true_heading(10.0, -5.0), 5.0);
        let wrapped = magnetometer::true_heading(2.0, -10.0);
        assert!((0.0..360.0).contains(&wrapped));
    }

    #[test]
    fn a_field_straight_up_or_down_has_no_heading() {
        let up = Vec3::new(0.0, 0.0, -9.8);
        assert_eq!(magnetometer::magnetic_heading(Vec3::new(0.0, 0.0, 40.0), up), None);
    }

    #[test]
    fn sea_level_pressure_is_zero_altitude() {
        let at_sea = altitude::absolute_altitude_m(altitude::STANDARD_SEA_LEVEL_KPA, altitude::STANDARD_SEA_LEVEL_KPA);
        assert!(at_sea.unwrap().abs() < 1e-9);
    }
}
