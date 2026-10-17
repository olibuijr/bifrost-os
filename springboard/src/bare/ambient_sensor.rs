//! Ambient light sensor on the IIO sysfs tree: finds the device with an illuminance channel and reads it as lux, then
//! turns the readings into a display brightness. The conversion and the lux-to-brightness rule are in
//! `bifrost_ui::ambient_light`; this module only reads the channel files. Nothing polls it yet: the shell has no
//! automatic brightness switch, so the reading is available to the shell but not applied to the panel.

use std::fs;
use std::path::{Path, PathBuf};

use bifrost_ui::ambient_light::{brightness_for_lux, lux_from_raw, ReadingError, Smoother};

const IIO_ROOT: &str = "/sys/bus/iio/devices";
/// Weight of each new reading. A small value hides the flicker of a passing shadow.
const SMOOTHING: f32 = 0.2;

/// The first IIO device (iio:device0, iio:device1, ...) with an illuminance channel. Entries are sorted, so the
/// pick is stable.
pub fn find_illuminance(root: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    dirs.sort();
    dirs.into_iter().find(|p| p.join("in_illuminance_raw").is_file())
}

/// Reads one lux value from an IIO device directory. The offset and the scale are optional channels; a missing one
/// takes the value the IIO ABI gives it (zero offset, unit scale).
pub fn read_lux(dir: &Path) -> Result<f32, ReadingError> {
    let channel = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
    let raw = fs::read_to_string(dir.join("in_illuminance_raw")).map_err(|_| ReadingError::Malformed)?;
    lux_from_raw(raw.trim(), channel("in_illuminance_offset").trim(), channel("in_illuminance_scale").trim())
}

/// The automatic brightness rule: lux readings, eased from one to the next, mapped to a panel brightness between
/// `MIN_BRIGHTNESS` and 1.
pub struct AutoBrightness {
    smoother: Smoother,
}

impl AutoBrightness {
    pub fn new() -> Self {
        AutoBrightness { smoother: Smoother::new(SMOOTHING).expect("SMOOTHING is in (0, 1]") }
    }

    /// Adds one lux reading and returns the brightness it calls for.
    pub fn push(&mut self, lux: f32) -> f32 {
        brightness_for_lux(self.smoother.push(lux))
    }
}

impl Default for AutoBrightness {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads the sensor under the IIO root, if the machine has one.
pub fn read_default() -> Option<f32> {
    let dir = find_illuminance(Path::new(IIO_ROOT))?;
    read_lux(&dir).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_tree(name: &str, devices: &[(&str, &[(&str, &str)])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ambient-sensor-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (dev, files) in devices {
            for (file, content) in *files {
                let p = root.join(dev).join(file);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, content).unwrap();
            }
        }
        root
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn finds_the_first_device_that_has_an_illuminance_channel() {
        let root = fake_tree(
            "find",
            &[
                ("iio:device0", &[("in_accel_x_raw", "1")]),
                ("iio:device1", &[("in_illuminance_raw", "5")]),
                ("iio:device2", &[("in_illuminance_raw", "9")]),
            ],
        );
        assert_eq!(find_illuminance(&root), Some(root.join("iio:device1")));
        assert_eq!(find_illuminance(Path::new("/nonexistent-springboard-ambient-test")), None);
    }

    #[test]
    fn a_machine_without_the_channel_has_no_sensor() {
        let root = fake_tree("none", &[("iio:device0", &[("in_accel_x_raw", "1")])]);
        assert_eq!(find_illuminance(&root), None);
    }

    #[test]
    fn read_lux_applies_the_offset_and_the_scale() {
        let root = fake_tree(
            "scale",
            &[("iio:device0", &[("in_illuminance_raw", "100\n"), ("in_illuminance_offset", "10\n"), ("in_illuminance_scale", "0.5\n")])],
        );
        assert!(near(read_lux(&root.join("iio:device0")).unwrap(), 55.0));
    }

    #[test]
    fn read_lux_without_scale_or_offset_uses_the_raw_count() {
        let root = fake_tree("plain", &[("iio:device0", &[("in_illuminance_raw", "42")])]);
        assert!(near(read_lux(&root.join("iio:device0")).unwrap(), 42.0));
    }

    #[test]
    fn read_lux_refuses_a_missing_or_bad_count() {
        let root = fake_tree(
            "bad",
            &[("iio:device0", &[("in_illuminance_scale", "1")]), ("iio:device1", &[("in_illuminance_raw", "lots")])],
        );
        assert_eq!(read_lux(&root.join("iio:device0")), Err(ReadingError::Malformed));
        assert_eq!(read_lux(&root.join("iio:device1")), Err(ReadingError::Malformed));
    }

    #[test]
    fn a_zero_scale_is_refused() {
        let root = fake_tree("zero", &[("iio:device0", &[("in_illuminance_raw", "3"), ("in_illuminance_scale", "0")])]);
        assert_eq!(read_lux(&root.join("iio:device0")), Err(ReadingError::OutOfRange));
    }

    #[test]
    fn brightness_follows_the_light_and_eases_toward_it() {
        let mut auto = AutoBrightness::new();
        let first = auto.push(10_000.0);
        assert!(near(first, 1.0), "the first reading is taken as it is");
        let dim = auto.push(1.0);
        assert!(dim < first, "a dark reading lowers the brightness");
        assert!(dim > brightness_for_lux(1.0), "the change is eased, not instant");
    }
}
