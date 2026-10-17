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
pub use bifrost_core::device_motion::DeviceMotionManager;
pub use bifrost_core::motion::{Attitude, DeviceMotion, MotionEstimator, Vec3};

/// The manifest permission an app declares to read device sensors (`permissions = ["sensors"]`). The shell grants it
/// to the app's policy; without it every sensor read is refused.
pub const SENSOR_PERMISSION: &str = "sensors";

/// The sensors an app can read through this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorKind {
    Magnetometer,
    Altimeter,
    MotionActivity,
    Pedometer,
    Attitude,
}

impl SensorKind {
    /// Every sensor, in the order the module lists them.
    pub const ALL: [SensorKind; 5] = [
        SensorKind::Magnetometer,
        SensorKind::Altimeter,
        SensorKind::MotionActivity,
        SensorKind::Pedometer,
        SensorKind::Attitude,
    ];

    /// The short name a log or an error message uses.
    pub fn name(self) -> &'static str {
        match self {
            SensorKind::Magnetometer => "magnetometer",
            SensorKind::Altimeter => "altimeter",
            SensorKind::MotionActivity => "motion-activity",
            SensorKind::Pedometer => "pedometer",
            SensorKind::Attitude => "attitude",
        }
    }
}

/// The sensors one app may read, decided from the permissions its manifest declares. It fails closed: an app that
/// does not declare [`SENSOR_PERMISSION`] reads nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorAccess {
    granted: bool,
}

impl SensorAccess {
    /// Access for an app that declares `permissions`.
    pub fn from_permissions<S: AsRef<str>>(permissions: &[S]) -> SensorAccess {
        SensorAccess { granted: permissions.iter().any(|p| p.as_ref() == SENSOR_PERMISSION) }
    }

    /// Whether the app may read `kind`. Every sensor needs the same permission.
    pub fn may_read(&self, kind: SensorKind) -> bool {
        let _ = kind;
        self.granted
    }

    /// The sensors the app may read, in [`SensorKind::ALL`] order.
    pub fn readable(&self) -> Vec<SensorKind> {
        SensorKind::ALL.into_iter().filter(|k| self.may_read(*k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_reads_need_the_sensors_permission() {
        let none: [&str; 0] = [];
        assert!(SensorAccess::from_permissions(&none).readable().is_empty());
        assert!(!SensorAccess::from_permissions(&["network"]).may_read(SensorKind::Pedometer));
        let granted = SensorAccess::from_permissions(&["network", SENSOR_PERMISSION]);
        assert_eq!(granted.readable(), SensorKind::ALL.to_vec());
        assert_eq!(SensorKind::MotionActivity.name(), "motion-activity");
    }

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
