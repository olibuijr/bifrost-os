# Core Motion parity (pure Rust models)

Official framework: https://developer.apple.com/documentation/coremotion (macOS 10.15 and later, through
Mac Catalyst). Rendered page evidence: REA `ev_8ab92618716263d274d95b15cad68b33b19435b75dd8401dfffea37ac23676da`.
The matrix row is `docs/parity-matrix/data/core-motion.json`.

This change adds three std-only models. They are the pure part of the framework: the arithmetic that turns raw
sensor samples into what an app would read. They are not yet wired to hardware or to an app API.

## What was built

| Module | What it does | Official names it mirrors (docs only) |
|---|---|---|
| `springboard/core/src/motion.rs` (`MotionEstimator`, `DeviceMotion`, `Attitude`, `Vec3`) | Complementary-filter attitude (roll, pitch from gravity and gyroscope; yaw from the gyroscope), unit quaternion, gravity estimate (one-second time constant), user acceleration (reading minus gravity), rotation rate. Non-finite samples are refused and keep the state. | CMDeviceMotion (attitude, rotationRate, gravity, userAcceleration), CMAttitude, CMAttitudeReferenceFrame (arbitrary frame only) |
| `springboard/core/src/sensor_log.rs` (`SensorLog<T>`) | Bounded, time-ordered sample log. Full logs drop the oldest sample. Out-of-order and non-finite times are refused. Range queries include both ends. `discard_before` trims history. | CMSensorRecorder.accelerometerData(from:to:), CMSensorDataList (model only) |
| `springboard/render/src/gfx/rotate.rs` (`orientation_from_gravity`) | Classifies a gravity reading into the four shell orientations. Gravity under 0.35 g is "flat" and returns None. A held orientation keeps a 15 degree hysteresis band past its quarter-circle, so a device near a boundary does not flip. | The orientation derived from CMAccelerometerData or CMDeviceMotion by the app |

Conventions (documented in the module headers):

- Readings are in g, and a device at rest reads one g along the axis that points up off the ground.
- Rates are in radians per second. Times are seconds on the caller's monotonic clock.
- Roll is zero with the screen flat face up and a quarter turn when the device stands upright in portrait.
- `orientation_from_gravity` takes gravity as seen on the screen: x runs right, y runs down, and the vector points toward the ground.

## Tests

Each module has its own `#[cfg(test)] mod tests`:

- `motion.rs`: rest reads no tilt; an upright device reads a quarter-turn roll; yaw integrates the gyroscope; the
  complementary filter bounds a constant gyroscope bias; user acceleration equals the reading minus the slow gravity
  estimate (closed form 0.2 / 1.1); non-finite samples keep the state; the quaternion is unit length and a quarter
  turn about x; yaw wraps into (-pi, pi].
- `sensor_log.rs`: a full log keeps the newest samples; out-of-order and non-finite times are refused; ranges include
  both ends and an inverted range is empty; `discard_before` trims; zero capacity keeps one sample.
- `gfx/rotate.rs`: each way the device is held; flat and non-finite readings give no orientation; the hysteresis band
  keeps the held side at 40 degrees; the switch happens past 60 degrees; the seam at upside-down wraps.

Per the workflow, nothing was built or run in this round. The next stage builds and runs `cargo test` for
`bifrost-core` and the `render` crate.

## Remaining gaps

- No app-facing API. Apps get no CMMotionManager-style object, no update delivery and no authorization model.
- No sensor feed. Nothing reads the accelerometer, gyroscope or magnetometer. `set_orientation` in
  `springboard/src/shell/rotate.rs` still has no caller outside its tests; `orientation_from_gravity` is the function
  that caller should use.
- Device motion has no rotation matrix, no reference frame other than an arbitrary yaw, and no
  magnetometer-corrected heading.
- No raw accelerometer, gyroscope or magnetometer events; no barometer or altitude.
- No motion-activity recognition and no pedometer or step counter.
- Sensor history is a model only: nothing records from hardware and there is no recording duration request.
