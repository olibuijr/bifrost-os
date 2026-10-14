//! The device motion manager: the app-facing shape of attitude, rotation rate and gravity estimates.
//!
//! An app asks whether device motion exists ([`DeviceMotionManager::is_available`]), sets how often it wants
//! estimates ([`DeviceMotionManager::set_update_interval`]), starts updates with a handler and reads the newest
//! estimate ([`DeviceMotionManager::device_motion`]). The estimate comes from [`MotionEstimator`], which fuses the
//! accelerometer and the gyroscope. This is the model the reference SDK calls a device motion manager; its
//! attitude reference frame is arbitrary, and yaw has no magnetometer to correct it.
//!
//! The manager owns no thread and no clock. The caller runs [`DeviceMotionManager::tick`] at its own cadence with
//! the current time and a [`SensorSource`]. Every tick that has both readings feeds the estimator, so integration
//! keeps its full sample rate; the handler is called only once the update interval has passed since the last
//! delivery. A tick with a missing reading, or a non-finite time, changes nothing.
//!
//! Decisions (ours): the interval defaults and limits are those of the raw motion manager; a non-finite or
//! non-positive interval is refused; starting without device motion returns `false`; starting a running manager
//! replaces its handler and restarts the estimate; stopping drops the handler, the newest estimate and the estimator
//! state.
use crate::motion::{DeviceMotion, MotionEstimator};
use crate::motion_manager::{SensorSource, DEFAULT_UPDATE_INTERVAL, MIN_UPDATE_INTERVAL};

/// Complementary blend used for the estimator: it suits readings at about 100 Hz.
const BLEND: f64 = 0.98;

/// Tolerance for float rounding when comparing the time since the last delivery with the interval, in seconds.
const TIME_EPSILON: f64 = 1e-9;

/// Starts, stops and reads the device motion estimate for an app.
pub struct DeviceMotionManager {
    available: bool,
    interval: f64,
    handler: Option<Box<dyn FnMut(DeviceMotion)>>,
    estimator: MotionEstimator,
    latest: Option<DeviceMotion>,
    last_delivered: Option<f64>,
}

impl DeviceMotionManager {
    /// A manager for a device that has, or has not, the sensors device motion needs. Nothing runs until started.
    pub fn new(available: bool) -> Self {
        DeviceMotionManager {
            available,
            interval: DEFAULT_UPDATE_INTERVAL,
            handler: None,
            estimator: MotionEstimator::new(BLEND),
            latest: None,
            last_delivered: None,
        }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    /// The update interval in seconds.
    pub fn update_interval(&self) -> f64 {
        self.interval
    }

    /// Sets the update interval in seconds. Returns `false` and keeps the old value when the interval is not finite
    /// or not positive. Intervals below the minimum are raised to it.
    pub fn set_update_interval(&mut self, seconds: f64) -> bool {
        if !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        self.interval = seconds.max(MIN_UPDATE_INTERVAL);
        true
    }

    /// Starts updates. Returns `false` when the device has no device motion.
    pub fn start(&mut self, handler: impl FnMut(DeviceMotion) + 'static) -> bool {
        if !self.available {
            return false;
        }
        self.handler = Some(Box::new(handler));
        self.estimator.reset();
        self.latest = None;
        self.last_delivered = None;
        true
    }

    /// Stops updates and drops the newest estimate.
    pub fn stop(&mut self) {
        self.handler = None;
        self.estimator.reset();
        self.latest = None;
        self.last_delivered = None;
    }

    pub fn is_active(&self) -> bool {
        self.handler.is_some()
    }

    /// The newest estimate, while updates run.
    pub fn device_motion(&self) -> Option<DeviceMotion> {
        if self.is_active() {
            self.latest
        } else {
            None
        }
    }

    /// Runs one pass at time `now` (seconds, monotonic). Does nothing when updates are not running, when `now` is not
    /// finite, or when either reading is missing. The estimator refuses non-finite readings itself.
    pub fn tick(&mut self, now: f64, source: &mut dyn SensorSource) {
        if !now.is_finite() || !self.is_active() {
            return;
        }
        let (accel, gyro) = match (source.accelerometer(), source.gyroscope()) {
            (Some(a), Some(g)) => (a, g),
            _ => return,
        };
        let Some(estimate) = self.estimator.update(now, accel, gyro) else {
            return;
        };
        self.latest = Some(estimate);
        let due = match self.last_delivered {
            None => true,
            Some(last) => now - last + TIME_EPSILON >= self.interval,
        };
        if due {
            self.last_delivered = Some(now);
            if let Some(handler) = self.handler.as_mut() {
                handler(estimate);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::Vec3;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A source with fixed readings; `None` simulates a missing sensor.
    struct Fixed {
        accel: Option<Vec3>,
        gyro: Option<Vec3>,
    }

    impl SensorSource for Fixed {
        fn accelerometer(&mut self) -> Option<Vec3> {
            self.accel
        }
        fn gyroscope(&mut self) -> Option<Vec3> {
            self.gyro
        }
    }

    fn flat() -> Fixed {
        Fixed { accel: Some(Vec3::new(0.0, 0.0, 1.0)), gyro: Some(Vec3::default()) }
    }

    fn counter() -> (Rc<Cell<u32>>, impl FnMut(DeviceMotion) + 'static) {
        let n = Rc::new(Cell::new(0));
        let c = Rc::clone(&n);
        (n, move |_| c.set(c.get() + 1))
    }

    #[test]
    fn a_device_without_device_motion_refuses_to_start() {
        let mut m = DeviceMotionManager::new(false);
        assert!(!m.is_available());
        assert!(!m.start(|_| {}));
        assert!(!m.is_active());
    }

    #[test]
    fn the_interval_is_checked_and_raised_to_the_minimum() {
        let mut m = DeviceMotionManager::new(true);
        assert_eq!(m.update_interval(), DEFAULT_UPDATE_INTERVAL);
        assert!(!m.set_update_interval(0.0));
        assert!(!m.set_update_interval(f64::NAN));
        assert!(!m.set_update_interval(-1.0));
        assert_eq!(m.update_interval(), DEFAULT_UPDATE_INTERVAL);
        assert!(m.set_update_interval(1e-6));
        assert_eq!(m.update_interval(), MIN_UPDATE_INTERVAL);
    }

    #[test]
    fn handler_is_called_once_per_interval_while_every_tick_integrates() {
        let mut m = DeviceMotionManager::new(true);
        let (n, handler) = counter();
        assert!(m.set_update_interval(0.1));
        assert!(m.start(handler));
        let mut src = flat();
        for i in 0..=20 {
            m.tick(i as f64 * 0.01, &mut src);
        }
        // deliveries at 0.00, 0.10 and 0.20
        assert_eq!(n.get(), 3);
        let d = m.device_motion().expect("an estimate while running");
        assert!((d.timestamp - 0.20).abs() < 1e-9);
        assert!(d.attitude.roll.abs() < 1e-9 && d.attitude.pitch.abs() < 1e-9);
    }

    #[test]
    fn a_missing_reading_or_a_bad_time_changes_nothing() {
        let mut m = DeviceMotionManager::new(true);
        m.start(|_| {});
        let mut src = Fixed { accel: None, gyro: Some(Vec3::default()) };
        m.tick(0.0, &mut src);
        assert!(m.device_motion().is_none());
        let mut ok = flat();
        m.tick(f64::NAN, &mut ok);
        assert!(m.device_motion().is_none());
        m.tick(0.0, &mut ok);
        assert!(m.device_motion().is_some());
    }

    #[test]
    fn stop_drops_the_handler_and_the_newest_estimate() {
        let mut m = DeviceMotionManager::new(true);
        let (n, handler) = counter();
        m.start(handler);
        m.tick(0.0, &mut flat());
        assert_eq!(n.get(), 1);
        m.stop();
        assert!(!m.is_active());
        assert!(m.device_motion().is_none());
        m.tick(1.0, &mut flat());
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn restarting_replaces_the_handler_and_the_estimate_starts_over() {
        let mut m = DeviceMotionManager::new(true);
        m.start(|_| {});
        let mut spin = Fixed { accel: Some(Vec3::new(0.0, 0.0, 1.0)), gyro: Some(Vec3::new(0.0, 0.0, 1.0)) };
        for i in 0..=10 {
            m.tick(i as f64 * 0.01, &mut spin);
        }
        assert!(m.device_motion().unwrap().attitude.yaw > 0.05);
        let (n, handler) = counter();
        assert!(m.start(handler));
        assert!(m.device_motion().is_none());
        m.tick(0.0, &mut flat());
        assert_eq!(n.get(), 1);
        assert!(m.device_motion().unwrap().attitude.yaw.abs() < 1e-9);
    }
}
