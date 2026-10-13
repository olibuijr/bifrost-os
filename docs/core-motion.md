# Core Motion: raw accelerometer and gyroscope

Reference: Apple Core Motion (`CMMotionManager`, `CMAccelerometerData`, `CMGyroData`). Matrix row data lives in
`docs/parity-matrix/data/core-motion.json`; this page records what BifrOSt built and the decisions taken.

## What is built

- `springboard/core/src/motion_manager.rs`: pure model with the `CMMotionManager` shape:
  `is_accelerometer_available` / `is_gyroscope_available`, `accelerometer_update_interval` /
  `gyroscope_update_interval` (seconds), `start_accelerometer(handler)` / `start_gyroscope(handler)`,
  `stop_*`, `is_*_active`, `accelerometer_data` / `gyroscope_data` (newest sample). The caller drives
  `tick(now, &mut dyn SensorSource)`; the model owns no thread and no clock.
- `springboard/core/src/inertial_sensors.rs`: reads the kernel's sensor tree (`/sys/bus/iio/devices`). It finds the
  first accelerometer and the first gyroscope by their channel files (`in_accel_{x,y,z}_raw`,
  `in_anglvel_{x,y,z}_raw`), applies `(raw + offset) * scale`, and converts accelerometer m/s^2 to g. `Sensors`
  implements `SensorSource`.
- SDK: `bifrost_app::{motion_manager, inertial_sensors}` re-export the same modules, so apps and the shell share one
  model.
- `springboard/sdk/src/motion_raw.rs`: `RawMotion` joins the two the way `CMMotionManager` does. It scans a sensor
  tree, starts accelerometer and gyroscope updates and snaps an update interval to a rate the sensor lists in
  `sampling_frequency_available` (`parse_frequencies`, `nearest_frequency`, `supported_frequencies`,
  `current_frequency`). A sensor without those files keeps the requested interval and is polled by `RawMotion::tick`.

## Units and frame

- Accelerometer: g, device frame (x right, y up the screen, z out of it). A flat device at rest reads (0, 0, -1)
  for gravity, the same convention as `motion::DeviceMotion`.
- Gyroscope: radians per second, same frame.
- Timestamps: seconds on the caller's monotonic clock.

## Decisions (ours)

- Update interval defaults to 0.1 s. Intervals below 1 ms are raised to 1 ms; zero, negative or non-finite values
  are refused and keep the old interval. Apple's own default and limits are not copied here and must be checked on
  a device.
- `tick` reads a sensor only while it runs. `latest` updates on every reading; the handler is called only when the
  interval has passed since the last delivery.
- Starting an unavailable sensor returns `false` and stores nothing. Starting a running sensor replaces its handler.
- Stopping a sensor drops its handler and its newest sample.
- No mounting matrix is applied: a device mounted rotated reads rotated.

## Not done

- Device motion: `springboard/core/src/device_motion.rs` (`DeviceMotionManager`) runs the `motion.rs` estimator on
  the accelerometer and gyroscope readings, with the same start, stop, interval and handler rules as the raw manager.
  SDK: `bifrost_app::sensors::DeviceMotionManager`. Every tick feeds the estimator; the handler runs at the interval.
  The shell does not read it yet, and the attitude frame has no magnetometer correction.
- Pedometer, altitude, magnetometer and motion activity are separate models (`core/src/{pedometer,altitude,magnetometer,motion_activity}.rs`, SDK `sensors.rs`) with their own rows. Sensor recording does not exist.
- Nothing is tested on the VM or on Midget yet. Host unit tests exist in both modules but were not run in this
  round, so the rows stay `partial` until a device test passes.
