//! Vehicle Motion Cues: a ring of dots at the screen edge that follows the motion of a moving vehicle, so the eyes see
//! what the inner ear feels. The shell feeds the accelerometer into the model in `bifrost_ui::motion_cues` and draws the
//! dots it returns.
//!
//! The feed reads the kernel's accelerometer (`bifrost_core::inertial_sensors`) every `POLL_MS`, takes gravity out with
//! the model's `GravityEstimate` and hands the remaining motion to `MotionCues`, which low-passes it. A device with no
//! accelerometer never shows a dot. Nothing is read while the cues are off.

use bifrost_core::inertial_sensors::Sensors;
use bifrost_ui::motion_cues::{Dot, GravityEstimate, MotionCues};

/// Milliseconds between two reads of the accelerometer (20 a second).
pub const POLL_MS: f64 = 50.0;
/// Seconds the gravity estimate takes to follow a push that lasts: a long steady turn fades from the cues.
const GRAVITY_SECONDS: f32 = 5.0;
/// A dot fainter than this is not drawn.
const VISIBLE: f32 = 0.01;

/// The accelerometer and the cue model behind the dots.
pub struct MotionCuesFeed {
    sensors: Sensors,
    gravity: GravityEstimate,
    cues: MotionCues,
    /// when the accelerometer was last read, in milliseconds
    last_ms: Option<f64>,
    /// the earliest time of the next read
    next_ms: f64,
    /// whether any dot was visible after the last read
    shown: bool,
}

impl MotionCuesFeed {
    /// The feed on the kernel's sensors, with the cues off.
    pub fn scan() -> Self {
        Self::with_sensors(Sensors::scan_default())
    }

    /// The feed on the given sensors, with the cues off.
    pub fn with_sensors(sensors: Sensors) -> Self {
        MotionCuesFeed {
            sensors,
            gravity: GravityEstimate::new(GRAVITY_SECONDS).expect("a positive time constant"),
            cues: MotionCues::new(false),
            last_ms: None,
            next_ms: f64::NEG_INFINITY,
            shown: false,
        }
    }

    /// Whether the machine has an accelerometer to follow.
    pub fn available(&self) -> bool {
        self.sensors.accelerometer.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.cues.is_enabled()
    }

    /// Turns the cues on or off. Off forgets the motion and the gravity estimate, so they start from rest when turned on.
    pub fn set_enabled(&mut self, on: bool) {
        if on == self.cues.is_enabled() {
            return;
        }
        self.cues.set_enabled(on);
        self.gravity = GravityEstimate::new(GRAVITY_SECONDS).expect("a positive time constant");
        self.last_ms = None;
        self.next_ms = f64::NEG_INFINITY;
        self.shown = false;
    }

    /// One shell tick at `t_ms`. The accelerometer is read at most every `POLL_MS` and only while the cues are on. True when
    /// the dots need drawing again: they are showing, or they just went out.
    pub fn tick(&mut self, t_ms: f64) -> bool {
        if !self.cues.is_enabled() || t_ms < self.next_ms {
            return false;
        }
        self.next_ms = t_ms + POLL_MS;
        let Some(g) = self.sensors.accelerometer.as_ref().and_then(|d| d.read()) else { return false };
        let dt = self.last_ms.replace(t_ms).map_or(POLL_MS / 1000.0, |prev| ((t_ms - prev) / 1000.0).max(0.001)) as f32;
        if let Some(motion) = self.gravity.linear([g.x as f32, g.y as f32, g.z as f32], dt) {
            self.cues.sample(motion, dt);
        }
        let was_shown = self.shown;
        self.shown = self.cues.dots().iter().any(|d| d.opacity > VISIBLE);
        self.shown || was_shown
    }

    /// Whether any dot is showing now.
    pub fn visible(&self) -> bool {
        self.shown
    }

    /// The dots to draw: those showing, with their place on the -1 to 1 square and their opacity.
    pub fn dots(&self) -> Vec<Dot> {
        self.cues.dots().into_iter().filter(|d| d.opacity > VISIBLE).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A sensor tree with one accelerometer whose raw counts are milli-g.
    fn tree(name: &str, raw: [i32; 3]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("motion-cues-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dev = root.join("iio:device0");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join("in_accel_scale"), "0.00980665").unwrap();
        set_raw(&root, raw);
        root
    }

    fn set_raw(root: &Path, raw: [i32; 3]) {
        for (axis, v) in ["x", "y", "z"].iter().zip(raw) {
            fs::write(root.join("iio:device0").join(format!("in_accel_{axis}_raw")), v.to_string()).unwrap();
        }
    }

    fn feed(root: &Path) -> MotionCuesFeed {
        let mut f = MotionCuesFeed::with_sensors(Sensors::scan(root));
        f.set_enabled(true);
        f
    }

    #[test]
    fn a_device_at_rest_shows_no_dot() {
        let root = tree("rest", [0, 0, 1000]);
        let mut f = feed(&root);
        assert!(f.available());
        for i in 0..40 {
            assert!(!f.tick(i as f64 * POLL_MS), "at rest nothing needs drawing");
        }
        assert!(!f.visible() && f.dots().is_empty());
    }

    #[test]
    fn a_sustained_push_lights_the_dots_on_its_side_then_they_go_out() {
        let root = tree("push", [0, 0, 1000]);
        let mut f = feed(&root);
        for i in 0..10 {
            f.tick(i as f64 * POLL_MS);
        }
        // 0.1 g toward +x: more than the full-strength threshold once the low-pass has caught up
        set_raw(&root, [100, 0, 1000]);
        let mut drawn = false;
        for i in 10..30 {
            drawn |= f.tick(i as f64 * POLL_MS);
        }
        assert!(drawn && f.visible());
        let dots = f.dots();
        let strongest = dots.iter().max_by(|a, b| a.opacity.total_cmp(&b.opacity)).expect("dots are showing");
        assert!(strongest.x > 0.8 && strongest.y.abs() < 0.2, "the dot toward +x is the brightest: {strongest:?}");
        assert!(dots.iter().all(|d| d.x > -0.5), "the far side stays dark: {dots:?}");
        // the push ends: the cues fade, and one more frame is asked for when they go out
        set_raw(&root, [0, 0, 1000]);
        let mut cleared = false;
        for i in 30..200 {
            let before = f.visible();
            let redraw = f.tick(i as f64 * POLL_MS);
            if before && !f.visible() {
                assert!(redraw, "the frame that clears the dots is drawn");
                cleared = true;
            }
        }
        assert!(cleared && f.dots().is_empty());
    }

    #[test]
    fn the_sensor_is_read_at_most_every_poll_interval() {
        let root = tree("rate", [0, 0, 1000]);
        let mut f = feed(&root);
        f.tick(0.0);
        // a push appears, but a tick inside the interval does not read it
        set_raw(&root, [900, 0, 1000]);
        assert!(!f.tick(POLL_MS / 2.0));
        assert!(!f.visible());
        f.tick(POLL_MS);
        assert!(f.visible(), "the next read sees the push");
    }

    #[test]
    fn nothing_is_read_while_the_cues_are_off_and_turning_them_off_clears_the_dots() {
        let root = tree("off", [0, 0, 1000]);
        let mut f = MotionCuesFeed::with_sensors(Sensors::scan(&root));
        assert!(!f.enabled());
        set_raw(&root, [900, 0, 1000]);
        assert!(!f.tick(0.0) && !f.visible(), "off: nothing is read");
        // switched on at rest, then a push
        set_raw(&root, [0, 0, 1000]);
        f.set_enabled(true);
        f.tick(100.0);
        set_raw(&root, [900, 0, 1000]);
        for i in 1..12 {
            f.tick(100.0 + i as f64 * POLL_MS);
        }
        assert!(f.visible() && !f.dots().is_empty());
        f.set_enabled(false);
        assert!(!f.visible() && f.dots().is_empty(), "switching off clears the dots");
        assert!(!f.tick(10_000.0), "off again: nothing is read");
    }

    #[test]
    fn a_machine_without_an_accelerometer_never_shows_a_dot() {
        let root = std::env::temp_dir().join(format!("motion-cues-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut f = feed(&root);
        assert!(!f.available());
        for i in 0..20 {
            assert!(!f.tick(i as f64 * POLL_MS));
        }
        assert!(f.dots().is_empty());
    }
}
