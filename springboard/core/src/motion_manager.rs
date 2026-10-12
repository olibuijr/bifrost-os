//! The motion manager: the app-facing shape of raw accelerometer and gyroscope updates.
//!
//! An app asks what the device has ([`Availability`]), sets how often it wants samples (an update interval per
//! sensor, in seconds), starts a sensor with a handler, and reads the latest sample. The model is the one an
//! Apple Core Motion app expects (`CMMotionManager`: `isAccelerometerAvailable`, `accelerometerUpdateInterval`,
//! `startAccelerometerUpdatesToQueue`-style handlers, `accelerometerData`, and the gyroscope pair).
//!
//! Units follow the sensors: accelerometer readings in g, gyroscope rates in radians per second, times in seconds on
//! a monotonic clock. The manager owns no thread and no clock. The caller runs [`MotionManager::tick`] at its own
//! cadence with the current time and a [`SensorSource`]; `tick` reads each running sensor, keeps the newest sample
//! and calls the handler once the update interval has passed since the last delivery. The kernel's sensor files are
//! read by [`crate::inertial_sensors`], which implements `SensorSource`.
//!
//! Decisions (ours, not copied from the reference): an update interval below [`MIN_UPDATE_INTERVAL`] is raised to
//! it; a non-finite or non-positive interval is refused and keeps the old value; starting a sensor that is not
//! available does nothing and returns `false`; starting a running sensor replaces its handler; stopping a sensor
//! drops its handler and its latest sample.
use crate::motion::Vec3;

/// The update interval a sensor starts with, in seconds (10 Hz).
pub const DEFAULT_UPDATE_INTERVAL: f64 = 0.1;

/// The fastest update interval accepted, in seconds (1 kHz).
pub const MIN_UPDATE_INTERVAL: f64 = 0.001;

/// Tolerance for float rounding when comparing the time since the last delivery with the interval, in seconds.
const TIME_EPSILON: f64 = 1e-9;

/// One accelerometer sample: the acceleration in g, with the device at rest reading one g along its up axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccelerometerData {
    pub timestamp: f64,
    pub acceleration: Vec3,
}

/// One gyroscope sample: the rotation rate in radians per second about each device axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GyroData {
    pub timestamp: f64,
    pub rotation_rate: Vec3,
}

/// Which raw sensors the device has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    pub accelerometer: bool,
    pub gyroscope: bool,
}

/// Where the manager reads its samples from. `None` means no reading now; the manager skips that sensor for the tick.
pub trait SensorSource {
    /// The accelerometer reading in g, in the device frame.
    fn accelerometer(&mut self) -> Option<Vec3>;
    /// The gyroscope reading in radians per second, in the device frame.
    fn gyroscope(&mut self) -> Option<Vec3>;
}

/// The state of one raw sensor: its interval, its handler, the newest sample and the last delivery time.
struct Channel<T: Copy> {
    available: bool,
    interval: f64,
    handler: Option<Box<dyn FnMut(T)>>,
    latest: Option<T>,
    last_delivered: Option<f64>,
}

impl<T: Copy> Channel<T> {
    fn new(available: bool) -> Self {
        Channel { available, interval: DEFAULT_UPDATE_INTERVAL, handler: None, latest: None, last_delivered: None }
    }

    fn is_running(&self) -> bool {
        self.handler.is_some()
    }

    fn set_interval(&mut self, seconds: f64) -> bool {
        if !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        self.interval = seconds.max(MIN_UPDATE_INTERVAL);
        true
    }

    fn start(&mut self, handler: Box<dyn FnMut(T)>) -> bool {
        if !self.available {
            return false;
        }
        self.handler = Some(handler);
        self.last_delivered = None;
        true
    }

    fn stop(&mut self) {
        self.handler = None;
        self.latest = None;
        self.last_delivered = None;
    }

    /// Keeps `make(now)` as the latest sample and delivers it if the interval has passed since the last delivery.
    fn offer(&mut self, now: f64, make: impl FnOnce(f64) -> T) {
        if !self.is_running() {
            return;
        }
        let sample = make(now);
        self.latest = Some(sample);
        let due = match self.last_delivered {
            None => true,
            Some(last) => now - last + TIME_EPSILON >= self.interval,
        };
        if due {
            self.last_delivered = Some(now);
            if let Some(handler) = self.handler.as_mut() {
                handler(sample);
            }
        }
    }
}

/// Starts, stops and reads the raw accelerometer and gyroscope for an app.
pub struct MotionManager {
    accelerometer: Channel<AccelerometerData>,
    gyroscope: Channel<GyroData>,
}

impl MotionManager {
    /// A manager for a device with the given sensors. Nothing runs until an app starts a sensor.
    pub fn new(availability: Availability) -> Self {
        MotionManager {
            accelerometer: Channel::new(availability.accelerometer),
            gyroscope: Channel::new(availability.gyroscope),
        }
    }

    /// Which sensors the device has.
    pub fn availability(&self) -> Availability {
        Availability { accelerometer: self.accelerometer.available, gyroscope: self.gyroscope.available }
    }

    pub fn is_accelerometer_available(&self) -> bool {
        self.accelerometer.available
    }

    pub fn is_gyroscope_available(&self) -> bool {
        self.gyroscope.available
    }

    /// The accelerometer update interval in seconds.
    pub fn accelerometer_update_interval(&self) -> f64 {
        self.accelerometer.interval
    }

    /// Sets the accelerometer update interval. Returns `false`, keeping the old value, for a non-finite or
    /// non-positive interval.
    pub fn set_accelerometer_update_interval(&mut self, seconds: f64) -> bool {
        self.accelerometer.set_interval(seconds)
    }

    /// The gyroscope update interval in seconds.
    pub fn gyroscope_update_interval(&self) -> f64 {
        self.gyroscope.interval
    }

    /// Sets the gyroscope update interval. Returns `false`, keeping the old value, for a non-finite or non-positive
    /// interval.
    pub fn set_gyroscope_update_interval(&mut self, seconds: f64) -> bool {
        self.gyroscope.set_interval(seconds)
    }

    /// Starts accelerometer updates. Returns `false` when the device has no accelerometer.
    pub fn start_accelerometer(&mut self, handler: impl FnMut(AccelerometerData) + 'static) -> bool {
        self.accelerometer.start(Box::new(handler))
    }

    pub fn stop_accelerometer(&mut self) {
        self.accelerometer.stop();
    }

    pub fn is_accelerometer_active(&self) -> bool {
        self.accelerometer.is_running()
    }

    /// The newest accelerometer sample, while the sensor runs.
    pub fn accelerometer_data(&self) -> Option<AccelerometerData> {
        self.accelerometer.latest
    }

    /// Starts gyroscope updates. Returns `false` when the device has no gyroscope.
    pub fn start_gyroscope(&mut self, handler: impl FnMut(GyroData) + 'static) -> bool {
        self.gyroscope.start(Box::new(handler))
    }

    pub fn stop_gyroscope(&mut self) {
        self.gyroscope.stop();
    }

    pub fn is_gyroscope_active(&self) -> bool {
        self.gyroscope.is_running()
    }

    /// The newest gyroscope sample, while the sensor runs.
    pub fn gyroscope_data(&self) -> Option<GyroData> {
        self.gyroscope.latest
    }

    /// Runs one pass at time `now` (seconds, monotonic). Only running sensors are read. A non-finite `now` does
    /// nothing. A reading that is not finite is skipped.
    pub fn tick(&mut self, now: f64, source: &mut dyn SensorSource) {
        if !now.is_finite() {
            return;
        }
        if self.accelerometer.is_running() {
            if let Some(g) = source.accelerometer().filter(|v| finite(*v)) {
                self.accelerometer.offer(now, |t| AccelerometerData { timestamp: t, acceleration: g });
            }
        }
        if self.gyroscope.is_running() {
            if let Some(r) = source.gyroscope().filter(|v| finite(*v)) {
                self.gyroscope.offer(now, |t| GyroData { timestamp: t, rotation_rate: r });
            }
        }
    }
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A source that returns fixed readings and counts how often each sensor was read.
    struct Fake {
        accel: Option<Vec3>,
        gyro: Option<Vec3>,
        accel_reads: u32,
        gyro_reads: u32,
    }

    impl Fake {
        fn new(accel: Option<Vec3>, gyro: Option<Vec3>) -> Self {
            Fake { accel, gyro, accel_reads: 0, gyro_reads: 0 }
        }
    }

    impl SensorSource for Fake {
        fn accelerometer(&mut self) -> Option<Vec3> {
            self.accel_reads += 1;
            self.accel
        }
        fn gyroscope(&mut self) -> Option<Vec3> {
            self.gyro_reads += 1;
            self.gyro
        }
    }

    fn both() -> Availability {
        Availability { accelerometer: true, gyroscope: true }
    }

    #[test]
    fn defaults_and_availability_are_reported() {
        let m = MotionManager::new(Availability { accelerometer: true, gyroscope: false });
        assert!(m.is_accelerometer_available());
        assert!(!m.is_gyroscope_available());
        assert_eq!(m.availability(), Availability { accelerometer: true, gyroscope: false });
        assert_eq!(m.accelerometer_update_interval(), DEFAULT_UPDATE_INTERVAL);
        assert_eq!(m.gyroscope_update_interval(), DEFAULT_UPDATE_INTERVAL);
        assert!(!m.is_accelerometer_active());
        assert_eq!(m.accelerometer_data(), None);
    }

    #[test]
    fn a_missing_sensor_refuses_to_start() {
        let mut m = MotionManager::new(Availability { accelerometer: false, gyroscope: false });
        assert!(!m.start_accelerometer(|_| {}));
        assert!(!m.start_gyroscope(|_| {}));
        assert!(!m.is_accelerometer_active());
        assert!(!m.is_gyroscope_active());
    }

    #[test]
    fn intervals_are_validated_and_clamped() {
        let mut m = MotionManager::new(both());
        assert!(m.set_accelerometer_update_interval(0.05));
        assert!((m.accelerometer_update_interval() - 0.05).abs() < 1e-12);
        // below the fastest rate is raised to it
        assert!(m.set_accelerometer_update_interval(0.0));
        assert_eq!(m.accelerometer_update_interval(), 0.05, "zero is refused and keeps the old value");
        assert!(m.set_gyroscope_update_interval(1e-9));
        assert_eq!(m.gyroscope_update_interval(), MIN_UPDATE_INTERVAL);
        // refused values keep the old interval
        assert!(!m.set_gyroscope_update_interval(f64::NAN));
        assert!(!m.set_gyroscope_update_interval(-1.0));
        assert_eq!(m.gyroscope_update_interval(), MIN_UPDATE_INTERVAL);
    }

    #[test]
    fn deliveries_follow_the_update_interval() {
        let mut m = MotionManager::new(both());
        m.set_accelerometer_update_interval(0.1);
        let got: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = got.clone();
        assert!(m.start_accelerometer(move |s| sink.borrow_mut().push(s.timestamp)));
        let mut src = Fake::new(Some(Vec3::new(0.0, 0.0, -1.0)), None);
        // a sensor sample every 20 ms for half a second: delivered every 100 ms
        for i in 0..=25 {
            m.tick(i as f64 * 0.02, &mut src);
        }
        let times = got.borrow();
        assert_eq!(times.len(), 6, "first sample at 0.0, then one per 0.1 s: {:?}", *times);
        assert!((times[1] - 0.10).abs() < 1e-9);
        assert!((times[5] - 0.50).abs() < 1e-9);
    }

    #[test]
    fn latest_tracks_every_sample_while_delivery_is_throttled() {
        let mut m = MotionManager::new(both());
        m.set_accelerometer_update_interval(1.0);
        assert!(m.start_accelerometer(|_| {}));
        let mut src = Fake::new(Some(Vec3::new(0.1, 0.2, 0.3)), None);
        m.tick(0.0, &mut src);
        m.tick(0.5, &mut src);
        let latest = m.accelerometer_data().unwrap();
        assert_eq!(latest.timestamp, 0.5);
        assert_eq!(latest.acceleration, Vec3::new(0.1, 0.2, 0.3));
    }

    #[test]
    fn stopped_sensors_are_not_read_and_drop_their_sample() {
        let mut m = MotionManager::new(both());
        assert!(m.start_gyroscope(|_| {}));
        let mut src = Fake::new(Some(Vec3::new(0.0, 0.0, -1.0)), Some(Vec3::new(0.0, 0.0, 0.5)));
        m.tick(1.0, &mut src);
        assert_eq!(src.accel_reads, 0, "the accelerometer was never started");
        assert_eq!(src.gyro_reads, 1);
        assert_eq!(m.gyroscope_data().unwrap().rotation_rate, Vec3::new(0.0, 0.0, 0.5));
        m.stop_gyroscope();
        assert!(!m.is_gyroscope_active());
        assert_eq!(m.gyroscope_data(), None);
        m.tick(2.0, &mut src);
        assert_eq!(src.gyro_reads, 1);
    }

    #[test]
    fn readings_that_are_not_finite_or_absent_are_skipped() {
        let mut m = MotionManager::new(both());
        let count = Rc::new(RefCell::new(0u32));
        let c = count.clone();
        assert!(m.start_accelerometer(move |_| *c.borrow_mut() += 1));
        let mut bad = Fake::new(Some(Vec3::new(f64::NAN, 0.0, 0.0)), None);
        m.tick(0.0, &mut bad);
        m.tick(f64::NAN, &mut bad);
        let mut none = Fake::new(None, None);
        m.tick(1.0, &mut none);
        assert_eq!(*count.borrow(), 0);
        assert_eq!(m.accelerometer_data(), None);
    }

    #[test]
    fn starting_again_replaces_the_handler() {
        let mut m = MotionManager::new(both());
        let first = Rc::new(RefCell::new(0u32));
        let second = Rc::new(RefCell::new(0u32));
        let (a, b) = (first.clone(), second.clone());
        assert!(m.start_accelerometer(move |_| *a.borrow_mut() += 1));
        assert!(m.start_accelerometer(move |_| *b.borrow_mut() += 1));
        let mut src = Fake::new(Some(Vec3::new(0.0, 0.0, -1.0)), None);
        m.tick(0.0, &mut src);
        assert_eq!(*first.borrow(), 0);
        assert_eq!(*second.borrow(), 1);
    }
}
