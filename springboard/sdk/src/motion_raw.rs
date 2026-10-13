//! Raw accelerometer and gyroscope updates over the kernel's sensor tree, with the device's sampling rates.
//!
//! [`crate::motion_manager`] holds the app-facing manager and [`crate::inertial_sensors`] reads the sensor files.
//! `RawMotion` joins them the way `CMMotionManager` does: it scans a sensor tree, starts accelerometer and gyroscope
//! updates (`startAccelerometerUpdates`, `startGyroUpdates`) and exposes the latest `accelerometerData` and
//! `gyroData`. An update interval is snapped to a rate the sensor supports: the kernel lists them in
//! `sampling_frequency_available` (hertz) and takes the chosen one in `sampling_frequency`. A sensor without those
//! files keeps the requested interval and is polled by [`RawMotion::tick`] at the caller's cadence.

use std::fs;
use std::path::{Path, PathBuf};

use crate::inertial_sensors::{Kind, Sensors};
use crate::motion_manager::{AccelerometerData, GyroData, MotionManager};

/// Parses a `sampling_frequency_available` listing: whitespace separated hertz values. Entries that are not finite
/// and positive are dropped; the result is sorted ascending. A range form (`[min step max]`) is not expanded and
/// yields nothing.
pub fn parse_frequencies(text: &str) -> Vec<f64> {
    // `[min step max]` is the IIO range form, not a list of rates
    if text.contains('[') {
        return Vec::new();
    }
    let mut list: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .filter(|f| f.is_finite() && *f > 0.0)
        .collect();
    list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    list
}

/// The listed rate closest to `wanted_hz`; the lower one on a tie. `None` for an empty list.
pub fn nearest_frequency(available: &[f64], wanted_hz: f64) -> Option<f64> {
    available.iter().copied().min_by(|a, b| (a - wanted_hz).abs().partial_cmp(&(b - wanted_hz).abs()).unwrap())
}

fn channel_prefix(kind: Kind) -> &'static str {
    match kind {
        Kind::Accelerometer => "in_accel_",
        Kind::Gyroscope => "in_anglvel_",
    }
}

/// Where a sensor's rate file lives: the channel's own file when it exists, else the device-wide one.
fn rate_file(dir: &Path, kind: Kind, suffix: &str) -> Option<PathBuf> {
    let own = dir.join(format!("{}sampling_frequency{}", channel_prefix(kind), suffix));
    let shared = dir.join(format!("sampling_frequency{}", suffix));
    [own, shared].into_iter().find(|p| p.is_file())
}

/// The sampling rates a sensor supports, in hertz.
pub fn supported_frequencies(dir: &Path, kind: Kind) -> Vec<f64> {
    rate_file(dir, kind, "_available")
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|t| parse_frequencies(&t))
        .unwrap_or_default()
}

/// The sampling rate the sensor runs at now, in hertz.
pub fn current_frequency(dir: &Path, kind: Kind) -> Option<f64> {
    let text = fs::read_to_string(rate_file(dir, kind, "")?).ok()?;
    text.trim().parse::<f64>().ok().filter(|f| f.is_finite() && *f > 0.0)
}

/// Picks the supported rate nearest to `seconds`, writes it to the sensor and returns the interval it gives. When
/// the sensor lists no rates, or the write fails, the requested interval stands.
fn apply_interval(dir: &Path, kind: Kind, seconds: f64) -> f64 {
    let list = supported_frequencies(dir, kind);
    let Some(hz) = nearest_frequency(&list, 1.0 / seconds) else {
        return seconds;
    };
    match rate_file(dir, kind, "").map(|p| fs::write(p, format!("{}", hz))) {
        Some(Ok(())) => 1.0 / hz,
        _ => seconds,
    }
}

/// A motion manager bound to the sensors found in one sensor tree.
pub struct RawMotion {
    manager: MotionManager,
    sensors: Sensors,
}

impl RawMotion {
    /// Scans `root` (normally [`crate::inertial_sensors::DEFAULT_ROOT`]).
    pub fn open(root: &Path) -> RawMotion {
        let sensors = Sensors::scan(root);
        RawMotion { manager: MotionManager::new(sensors.availability()), sensors }
    }

    pub fn is_accelerometer_available(&self) -> bool {
        self.manager.is_accelerometer_available()
    }

    pub fn is_gyroscope_available(&self) -> bool {
        self.manager.is_gyroscope_available()
    }

    pub fn accelerometer_update_interval(&self) -> f64 {
        self.manager.accelerometer_update_interval()
    }

    pub fn gyroscope_update_interval(&self) -> f64 {
        self.manager.gyroscope_update_interval()
    }

    /// Sets the accelerometer interval, snapped to a rate the sensor supports. `false` for a non-finite or
    /// non-positive value, or when there is no accelerometer.
    pub fn set_accelerometer_update_interval(&mut self, seconds: f64) -> bool {
        let Some(device) = self.sensors.accelerometer.as_ref() else {
            return false;
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        let snapped = apply_interval(&device.path, Kind::Accelerometer, seconds);
        self.manager.set_accelerometer_update_interval(snapped)
    }

    /// Sets the gyroscope interval, snapped like the accelerometer's.
    pub fn set_gyroscope_update_interval(&mut self, seconds: f64) -> bool {
        let Some(device) = self.sensors.gyroscope.as_ref() else {
            return false;
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        let snapped = apply_interval(&device.path, Kind::Gyroscope, seconds);
        self.manager.set_gyroscope_update_interval(snapped)
    }

    /// `startAccelerometerUpdates`: `false` when the device has no accelerometer.
    pub fn start_accelerometer_updates(&mut self) -> bool {
        self.manager.start_accelerometer(|_| {})
    }

    /// `startAccelerometerUpdatesToQueue` with a handler.
    pub fn start_accelerometer_updates_with(&mut self, handler: impl FnMut(AccelerometerData) + 'static) -> bool {
        self.manager.start_accelerometer(handler)
    }

    pub fn stop_accelerometer_updates(&mut self) {
        self.manager.stop_accelerometer();
    }

    pub fn is_accelerometer_active(&self) -> bool {
        self.manager.is_accelerometer_active()
    }

    /// `accelerometerData`: the newest sample, `None` before the first tick or when stopped.
    pub fn accelerometer_data(&self) -> Option<AccelerometerData> {
        self.manager.accelerometer_data()
    }

    /// `startGyroUpdates`.
    pub fn start_gyro_updates(&mut self) -> bool {
        self.manager.start_gyroscope(|_| {})
    }

    /// `startGyroUpdatesToQueue` with a handler.
    pub fn start_gyro_updates_with(&mut self, handler: impl FnMut(GyroData) + 'static) -> bool {
        self.manager.start_gyroscope(handler)
    }

    pub fn stop_gyro_updates(&mut self) {
        self.manager.stop_gyroscope();
    }

    pub fn is_gyro_active(&self) -> bool {
        self.manager.is_gyroscope_active()
    }

    /// `gyroData`: the newest sample.
    pub fn gyro_data(&self) -> Option<GyroData> {
        self.manager.gyroscope_data()
    }

    /// Reads each running sensor at monotonic time `now` (seconds) and delivers due samples.
    pub fn tick(&mut self, now: f64) {
        self.manager.tick(now, &mut self.sensors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inertial_sensors::STANDARD_GRAVITY;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("motion-raw-{}-{}-{}", tag, std::process::id(), nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A combined unit: accelerometer at 1 g per 2048 counts, gyroscope 0.001 rad/s per count, shared rates.
    fn fixture(tag: &str) -> PathBuf {
        let root = scratch(tag);
        let dev = root.join("iio:device0");
        fs::create_dir_all(&dev).unwrap();
        for (axis, raw) in [("x", "0"), ("y", "0"), ("z", "2048")] {
            fs::write(dev.join(format!("in_accel_{}_raw", axis)), raw).unwrap();
        }
        for (axis, raw) in [("x", "100"), ("y", "-200"), ("z", "0")] {
            fs::write(dev.join(format!("in_anglvel_{}_raw", axis)), raw).unwrap();
        }
        fs::write(dev.join("in_accel_scale"), format!("{}", STANDARD_GRAVITY / 2048.0)).unwrap();
        fs::write(dev.join("in_anglvel_scale"), "0.001").unwrap();
        fs::write(dev.join("sampling_frequency_available"), "12.5 25 50 100\n").unwrap();
        fs::write(dev.join("sampling_frequency"), "25\n").unwrap();
        root
    }

    #[test]
    fn frequencies_are_parsed_and_sorted() {
        assert_eq!(parse_frequencies("100 12.5 50\n25"), vec![12.5, 25.0, 50.0, 100.0]);
        assert_eq!(parse_frequencies("0 -5 nan inf x 10"), vec![10.0]);
        assert_eq!(parse_frequencies("[1 1 100]"), Vec::<f64>::new());
        assert_eq!(parse_frequencies(""), Vec::<f64>::new());
    }

    #[test]
    fn the_nearest_listed_rate_wins_and_ties_go_low() {
        let list = [12.5, 25.0, 50.0, 100.0];
        assert_eq!(nearest_frequency(&list, 60.0), Some(50.0));
        assert_eq!(nearest_frequency(&list, 1000.0), Some(100.0));
        assert_eq!(nearest_frequency(&list, 37.5), Some(25.0));
        assert_eq!(nearest_frequency(&[], 10.0), None);
    }

    #[test]
    fn a_fixture_tree_gives_samples_in_g_and_radians() {
        let root = fixture("samples");
        let mut m = RawMotion::open(&root);
        assert!(m.is_accelerometer_available() && m.is_gyroscope_available());
        assert!(m.accelerometer_data().is_none());
        assert!(m.start_accelerometer_updates());
        assert!(m.start_gyro_updates());
        m.tick(1.0);
        let a = m.accelerometer_data().unwrap();
        assert_eq!(a.timestamp, 1.0);
        assert!((a.acceleration.z - 1.0).abs() < 1e-9 && a.acceleration.x == 0.0);
        let g = m.gyro_data().unwrap();
        assert!((g.rotation_rate.x - 0.1).abs() < 1e-9);
        assert!((g.rotation_rate.y + 0.2).abs() < 1e-9);
        m.stop_accelerometer_updates();
        assert!(m.accelerometer_data().is_none());
        assert!(!m.is_accelerometer_active() && m.is_gyro_active());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_interval_snaps_to_a_supported_rate_and_is_written_to_the_sensor() {
        let root = fixture("interval");
        let dev = root.join("iio:device0");
        assert_eq!(supported_frequencies(&dev, Kind::Accelerometer), vec![12.5, 25.0, 50.0, 100.0]);
        assert_eq!(current_frequency(&dev, Kind::Accelerometer), Some(25.0));
        let mut m = RawMotion::open(&root);
        assert!(m.set_accelerometer_update_interval(0.018)); // about 55 Hz
        assert!((m.accelerometer_update_interval() - 0.02).abs() < 1e-12);
        assert_eq!(current_frequency(&dev, Kind::Accelerometer), Some(50.0));
        assert!(!m.set_gyroscope_update_interval(0.0));
        assert!(!m.set_gyroscope_update_interval(f64::NAN));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn handlers_follow_the_update_interval() {
        let root = fixture("handler");
        let mut m = RawMotion::open(&root);
        assert!(m.set_accelerometer_update_interval(0.1)); // snaps to 12.5 Hz = 0.08 s
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        assert!(m.start_accelerometer_updates_with(move |d| sink.borrow_mut().push(d.timestamp)));
        for t in [0.0, 0.04, 0.08, 0.12, 0.16] {
            m.tick(t);
        }
        assert_eq!(*seen.borrow(), vec![0.0, 0.08, 0.16]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_sensor_without_rate_files_keeps_the_requested_interval() {
        let root = scratch("norate");
        let dev = root.join("iio:device1");
        fs::create_dir_all(&dev).unwrap();
        for axis in ["x", "y", "z"] {
            fs::write(dev.join(format!("in_anglvel_{}_raw", axis)), "1").unwrap();
        }
        fs::write(dev.join("in_anglvel_scale"), "0.01").unwrap();
        let mut m = RawMotion::open(&root);
        assert!(!m.is_accelerometer_available());
        assert!(!m.start_accelerometer_updates());
        assert!(!m.set_accelerometer_update_interval(0.1));
        assert!(m.set_gyroscope_update_interval(0.05));
        assert!((m.gyroscope_update_interval() - 0.05).abs() < 1e-12);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn per_channel_rate_files_win_over_the_shared_one() {
        let root = fixture("channel");
        let dev = root.join("iio:device0");
        fs::write(dev.join("in_anglvel_sampling_frequency_available"), "200 400").unwrap();
        fs::write(dev.join("in_anglvel_sampling_frequency"), "200").unwrap();
        assert_eq!(supported_frequencies(&dev, Kind::Gyroscope), vec![200.0, 400.0]);
        assert_eq!(supported_frequencies(&dev, Kind::Accelerometer), vec![12.5, 25.0, 50.0, 100.0]);
        let mut m = RawMotion::open(&root);
        assert!(m.set_gyroscope_update_interval(0.002));
        assert_eq!(current_frequency(&dev, Kind::Gyroscope), Some(400.0));
        assert_eq!(current_frequency(&dev, Kind::Accelerometer), Some(25.0));
        fs::remove_dir_all(&root).ok();
    }
}
