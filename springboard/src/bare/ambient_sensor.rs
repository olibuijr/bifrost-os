//! Ambient light sensor on the IIO sysfs tree: finds the device with an illuminance channel and reads it as lux, then
//! turns the readings into a display brightness. The conversion and the lux-to-brightness rule are in
//! `bifrost_ui::ambient_light`; this module only reads the channel files. `poll_brightness` is one poll: it reads the
//! sensor, smooths the reading and returns a panel brightness only when it has moved far enough to be worth writing.

use std::fs;
use std::path::{Path, PathBuf};

use bifrost_ui::ambient_light::{brightness_for_lux, lux_from_raw, BrightnessGate, ReadingError, Smoother};

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

/// Whether the machine has an ambient light sensor.
pub fn sensor_present() -> bool {
    find_illuminance(Path::new(IIO_ROOT)).is_some()
}

/// Reads the sensor under the IIO root, if the machine has one.
pub fn read_default() -> Option<f32> {
    let dir = find_illuminance(Path::new(IIO_ROOT))?;
    read_lux(&dir).ok()
}

/// One automatic brightness poll under `root`: reads the sensor, folds the lux into `auto` and returns the brightness
/// to write, when `gate` lets one through. A machine without a sensor, or a bad reading, returns `None`.
pub fn poll_brightness(root: &Path, auto: &mut AutoBrightness, gate: &mut BrightnessGate) -> Option<f32> {
    let dir = find_illuminance(root)?;
    let lux = read_lux(&dir).ok()?;
    gate.update(auto.push(lux))
}

/// Milliseconds between two reads of the sensor.
pub const POLL_MS: f64 = 2000.0;
/// How far the brightness must move before the panel is written again (a fraction of full brightness).
const STEP: f32 = 0.05;

/// Automatic brightness as the shell runs it: the sensor is read at most every `POLL_MS`, smoothed, and a level comes back
/// only when it has moved `STEP` from the one last written.
pub struct AutoLight {
    auto: AutoBrightness,
    gate: BrightnessGate,
    next_ms: f64,
}

impl AutoLight {
    pub fn new() -> Self {
        AutoLight { auto: AutoBrightness::new(), gate: BrightnessGate::new(STEP).expect("STEP is in (0, 1]"), next_ms: f64::NEG_INFINITY }
    }

    /// One turn of the loop at `now_ms` over the sensors under `root`: None before `POLL_MS` have passed since the last
    /// read, and when the light has not moved the brightness far enough; otherwise the level to write.
    pub fn poll(&mut self, root: &Path, now_ms: f64) -> Option<f32> {
        if now_ms < self.next_ms {
            return None;
        }
        self.next_ms = now_ms + POLL_MS;
        poll_brightness(root, &mut self.auto, &mut self.gate)
    }

    /// `poll` on the machine's own sensors.
    pub fn poll_default(&mut self, now_ms: f64) -> Option<f32> {
        self.poll(Path::new(IIO_ROOT), now_ms)
    }

    /// Forgets the level last written, so the next reading is written whatever it is (automatic brightness was just
    /// switched on, or the screen woke).
    pub fn restart(&mut self) {
        self.gate = BrightnessGate::new(STEP).expect("STEP is in (0, 1]");
        self.next_ms = f64::NEG_INFINITY;
    }
}

impl Default for AutoLight {
    fn default() -> Self {
        Self::new()
    }
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

    #[test]
    fn a_poll_writes_the_first_level_then_only_real_changes() {
        let root = fake_tree("poll", &[("iio:device0", &[("in_illuminance_raw", "10000")])]);
        let mut auto = AutoBrightness::new();
        let mut gate = BrightnessGate::new(0.05).unwrap();
        let first = poll_brightness(&root, &mut auto, &mut gate).expect("the first poll writes");
        assert!(near(first, 1.0));
        assert_eq!(poll_brightness(&root, &mut auto, &mut gate), None, "the same light writes nothing");
    }

    #[test]
    fn a_poll_without_a_sensor_writes_nothing() {
        let root = fake_tree("poll-none", &[("iio:device0", &[("in_accel_x_raw", "1")])]);
        let mut auto = AutoBrightness::new();
        let mut gate = BrightnessGate::new(0.05).unwrap();
        assert_eq!(poll_brightness(&root, &mut auto, &mut gate), None);
        assert_eq!(gate.applied(), None);
    }

    #[test]
    fn the_sensor_is_read_every_two_seconds_and_a_level_comes_back_only_for_a_real_change() {
        let root = fake_tree("autolight", &[("iio:device0", &[("in_illuminance_raw", "10000")])]);
        let mut light = AutoLight::new();
        assert!(near(light.poll(&root, 0.0).expect("the first reading is written"), 1.0));
        fs::write(root.join("iio:device0").join("in_illuminance_raw"), "1").unwrap();
        assert_eq!(light.poll(&root, POLL_MS - 1.0), None, "the sensor is not read again yet");
        // the room goes dark: the level eases down over several readings and is written once it has moved a step
        let mut written = Vec::new();
        for i in 1..=40 {
            if let Some(level) = light.poll(&root, i as f64 * POLL_MS) {
                written.push(level);
            }
        }
        assert!(!written.is_empty() && written[0] <= 0.951, "{written:?}");
        assert!(written.windows(2).all(|w| w[1] < w[0]), "it only gets darker: {written:?}");
        assert!(written.len() < 20, "writes are a step apart, not one per reading: {written:?}");
        assert!(written.iter().all(|l| *l >= bifrost_ui::ambient_light::MIN_BRIGHTNESS), "{written:?}");
    }

    #[test]
    fn restarting_writes_the_next_reading_at_once() {
        let root = fake_tree("autolight-restart", &[("iio:device0", &[("in_illuminance_raw", "10000")])]);
        let mut light = AutoLight::new();
        assert!(light.poll(&root, 0.0).is_some());
        assert_eq!(light.poll(&root, POLL_MS), None, "the same light writes nothing");
        light.restart();
        assert!(near(light.poll(&root, POLL_MS + 1.0).expect("written again"), 1.0));
    }

    #[test]
    fn a_machine_without_a_sensor_never_gets_a_level() {
        let root = fake_tree("autolight-none", &[("iio:device0", &[("in_accel_x_raw", "1")])]);
        let mut light = AutoLight::new();
        assert_eq!(light.poll(&root, 0.0), None);
        assert_eq!(light.poll(&root, 10.0 * POLL_MS), None);
    }
}
