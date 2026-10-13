//! Reads the accelerometer and gyroscope from the kernel's sensor tree, when the device has one.
//!
//! The kernel exposes each motion sensor as a device under `/sys/bus/iio/devices/iio:deviceN`, with one file per
//! channel. An accelerometer has `in_accel_x_raw`, `in_accel_y_raw`, `in_accel_z_raw`; a gyroscope has the same
//! names with `in_anglvel_`. A channel value is `(raw + offset) * scale`, where `offset` and `scale` come from the
//! `_offset` and `_scale` files of the axis, or the shared file without an axis. Accelerometer scale is in m/s^2 per
//! raw unit and is converted to g here; gyroscope scale is already in radians per second.
//!
//! Axes follow the sensor's own mounting. This module applies no mounting matrix, so a device mounted rotated
//! reads rotated. A device that is absent or unreadable is simply not available; nothing here panics.
use std::fs;
use std::path::{Path, PathBuf};

use crate::motion::Vec3;
use crate::motion_manager::{Availability, SensorSource};

/// Where the kernel lists its sensor devices.
pub const DEFAULT_ROOT: &str = "/sys/bus/iio/devices";

/// Standard gravity in m/s^2, the scale from the accelerometer's m/s^2 to g.
pub const STANDARD_GRAVITY: f64 = 9.80665;

const AXES: [char; 3] = ['x', 'y', 'z'];

/// The two sensor kinds this module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Accelerometer,
    Gyroscope,
}

impl Kind {
    /// The channel name prefix of the kind's files.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Accelerometer => "in_accel_",
            Kind::Gyroscope => "in_anglvel_",
        }
    }
}

/// Which kinds a device exposes, from the names of its files. A device with both (a combined motion unit) yields
/// both, accelerometer first.
pub fn kinds_in(files: &[&str]) -> Vec<Kind> {
    let has_axes = |kind: Kind| {
        AXES.iter().all(|axis| {
            let raw = format!("{}{}_raw", kind.prefix(), axis);
            files.iter().any(|f| *f == raw)
        })
    };
    let mut kinds = Vec::new();
    if has_axes(Kind::Accelerometer) {
        kinds.push(Kind::Accelerometer);
    }
    if has_axes(Kind::Gyroscope) {
        kinds.push(Kind::Gyroscope);
    }
    kinds
}

/// Parses a sysfs number. Returns `None` for text that is not a finite number.
pub fn parse_number(text: &str) -> Option<f64> {
    let v: f64 = text.trim().parse().ok()?;
    v.is_finite().then_some(v)
}

/// Converts raw channel values to the sensor's unit: `(raw + offset) * scale`, with the accelerometer divided by
/// standard gravity so that it reads in g.
pub fn convert(kind: Kind, raw: [f64; 3], offset: [f64; 3], scale: [f64; 3]) -> Vec3 {
    let value = |i: usize| (raw[i] + offset[i]) * scale[i];
    let v = Vec3::new(value(0), value(1), value(2));
    match kind {
        Kind::Accelerometer => Vec3::new(v.x / STANDARD_GRAVITY, v.y / STANDARD_GRAVITY, v.z / STANDARD_GRAVITY),
        Kind::Gyroscope => v,
    }
}

/// One sensor device, opened: its directory and its per-axis offset and scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub path: PathBuf,
    pub kind: Kind,
    offset: [f64; 3],
    scale: [f64; 3],
}

fn read_number(path: &Path) -> Option<f64> {
    parse_number(&fs::read_to_string(path).ok()?)
}

impl Device {
    /// Opens the device at `path` for `kind`. Returns `None` when a raw axis file is missing or the scale is absent:
    /// without a scale the raw counts cannot be turned into a unit. A missing offset is zero.
    pub fn open(path: &Path, kind: Kind) -> Option<Device> {
        let prefix = kind.prefix();
        let shared_scale = read_number(&path.join(format!("{}scale", prefix)));
        let shared_offset = read_number(&path.join(format!("{}offset", prefix)));
        let mut offset = [0.0; 3];
        let mut scale = [0.0; 3];
        for (i, axis) in AXES.iter().enumerate() {
            if !path.join(format!("{}{}_raw", prefix, axis)).is_file() {
                return None;
            }
            scale[i] = read_number(&path.join(format!("{}{}_scale", prefix, axis)))
                .or(shared_scale)?;
            offset[i] = read_number(&path.join(format!("{}{}_offset", prefix, axis)))
                .or(shared_offset)
                .unwrap_or(0.0);
        }
        Some(Device { path: path.to_path_buf(), kind, offset, scale })
    }

    /// Reads one sample now, in g for the accelerometer and in radians per second for the gyroscope. `None` on any
    /// read or parse failure.
    pub fn read(&self) -> Option<Vec3> {
        let prefix = self.kind.prefix();
        let mut raw = [0.0; 3];
        for (i, axis) in AXES.iter().enumerate() {
            raw[i] = read_number(&self.path.join(format!("{}{}_raw", prefix, axis)))?;
        }
        Some(convert(self.kind, raw, self.offset, self.scale))
    }
}

/// The motion sensors found in a sensor tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sensors {
    pub accelerometer: Option<Device>,
    pub gyroscope: Option<Device>,
}

impl Sensors {
    /// Scans `root` for the first usable accelerometer and gyroscope. Devices are visited in name order, so the
    /// choice is stable. A missing or unreadable root gives no sensors.
    pub fn scan(root: &Path) -> Sensors {
        let mut sensors = Sensors::default();
        let Ok(entries) = fs::read_dir(root) else {
            return sensors;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("iio:device")))
            .collect();
        dirs.sort();
        for dir in dirs {
            let names: Vec<String> = match fs::read_dir(&dir) {
                Ok(files) => files.filter_map(|f| f.ok()?.file_name().into_string().ok()).collect(),
                Err(_) => continue,
            };
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            for kind in kinds_in(&refs) {
                match kind {
                    Kind::Accelerometer if sensors.accelerometer.is_none() => {
                        sensors.accelerometer = Device::open(&dir, kind);
                    }
                    Kind::Gyroscope if sensors.gyroscope.is_none() => {
                        sensors.gyroscope = Device::open(&dir, kind);
                    }
                    _ => {}
                }
            }
        }
        sensors
    }

    /// Scans the kernel's sensor tree at [`DEFAULT_ROOT`].
    pub fn scan_default() -> Sensors {
        Sensors::scan(Path::new(DEFAULT_ROOT))
    }

    /// Which sensors were found.
    pub fn availability(&self) -> Availability {
        Availability { accelerometer: self.accelerometer.is_some(), gyroscope: self.gyroscope.is_some() }
    }
}

impl SensorSource for Sensors {
    fn accelerometer(&mut self) -> Option<Vec3> {
        self.accelerometer.as_ref()?.read()
    }

    fn gyroscope(&mut self) -> Option<Vec3> {
        self.gyroscope.as_ref()?.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: Vec3, b: Vec3) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
    }

    #[test]
    fn kinds_follow_the_channel_files() {
        let accel = ["name", "in_accel_x_raw", "in_accel_y_raw", "in_accel_z_raw", "in_accel_scale"];
        assert_eq!(kinds_in(&accel), vec![Kind::Accelerometer]);
        let gyro = ["in_anglvel_x_raw", "in_anglvel_y_raw", "in_anglvel_z_raw"];
        assert_eq!(kinds_in(&gyro), vec![Kind::Gyroscope]);
        let both = [
            "in_accel_x_raw", "in_accel_y_raw", "in_accel_z_raw",
            "in_anglvel_x_raw", "in_anglvel_y_raw", "in_anglvel_z_raw",
        ];
        assert_eq!(kinds_in(&both), vec![Kind::Accelerometer, Kind::Gyroscope]);
        // a partial set of axes is not a usable sensor
        assert_eq!(kinds_in(&["in_accel_x_raw", "in_accel_y_raw"]), Vec::<Kind>::new());
    }

    #[test]
    fn parse_number_refuses_text_that_is_not_finite() {
        assert_eq!(parse_number(" 12\n"), Some(12.0));
        assert_eq!(parse_number("-0.5"), Some(-0.5));
        assert_eq!(parse_number("nan"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("abc"), None);
    }

    #[test]
    fn accelerometer_counts_become_g() {
        // a scale of standard gravity per count reads one g per count
        let g = convert(Kind::Accelerometer, [0.0, 0.0, 1.0], [0.0; 3], [STANDARD_GRAVITY; 3]);
        assert!(same(g, Vec3::new(0.0, 0.0, 1.0)));
        // offset is added before scale: (2 + 3) * 0.5 * g
        let g = convert(Kind::Accelerometer, [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.5 * STANDARD_GRAVITY; 3]);
        assert!(same(g, Vec3::new(2.5, 0.0, 0.0)));
    }

    #[test]
    fn gyroscope_scale_is_already_radians_per_second() {
        let r = convert(Kind::Gyroscope, [100.0, -100.0, 0.0], [0.0; 3], [0.001, 0.001, 0.001]);
        assert!(same(r, Vec3::new(0.1, -0.1, 0.0)));
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("motion-{}-{}-{}", tag, std::process::id(), nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_finds_an_accelerometer_and_reads_it() {
        let root = scratch_dir("scan");
        let dev = root.join("iio:device0");
        fs::create_dir_all(&dev).unwrap();
        for (axis, raw) in [("x", "0"), ("y", "0"), ("z", "2048")] {
            fs::write(dev.join(format!("in_accel_{}_raw", axis)), raw).unwrap();
        }
        // one shared scale: 1 g per 2048 counts, in m/s^2 per count
        fs::write(dev.join("in_accel_scale"), format!("{}\n", STANDARD_GRAVITY / 2048.0)).unwrap();

        let mut sensors = Sensors::scan(&root);
        assert_eq!(sensors.availability(), Availability { accelerometer: true, gyroscope: false });
        let reading = sensors.accelerometer().expect("the accelerometer reads");
        assert!(same(reading, Vec3::new(0.0, 0.0, 1.0)));
        assert_eq!(sensors.gyroscope(), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_of_a_missing_root_finds_nothing() {
        let root = std::env::temp_dir().join("motion-no-such-root-for-tests");
        let sensors = Sensors::scan(&root);
        assert_eq!(sensors, Sensors::default());
        assert_eq!(sensors.availability(), Availability::default());
    }

    #[test]
    fn a_device_without_a_scale_is_not_opened() {
        let root = scratch_dir("noscale");
        let dev = root.join("iio:device1");
        fs::create_dir_all(&dev).unwrap();
        for axis in ["x", "y", "z"] {
            fs::write(dev.join(format!("in_anglvel_{}_raw", axis)), "5").unwrap();
        }
        assert!(Device::open(&dev, Kind::Gyroscope).is_none());
        fs::write(dev.join("in_anglvel_scale"), "0.01").unwrap();
        let device = Device::open(&dev, Kind::Gyroscope).expect("opens once the scale exists");
        assert!(same(device.read().unwrap(), Vec3::new(0.05, 0.05, 0.05)));
        fs::remove_dir_all(&root).ok();
    }
}
