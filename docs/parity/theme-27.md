# Theme 27: motion sensing (gyro and accelerometer)

Pure models for the parts of motion sensing that do not need a sensor: magnetic headings, altitude from pressure,
steps and distance, a coarse activity label, the rumble a controller takes, and the rumble capability bit of a
force-feedback node. The module set is in `springboard/core/src/`. No sensor is read, no device is opened and no
SDK call is added, so every row below is `partial`: the models exist, but no app can reach them yet.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `magnetometer.rs` | `CMMagnetometerData` (field), the heading part of `CLHeading` | `magnetic_heading(field, gravity)` tilts the field back to the horizontal plane and returns the top edge's heading in degrees clockwise from magnetic north, 0..360, or `None` when the device points straight up or down. `true_heading` applies a caller-supplied declination (east positive). `Calibration` learns the hard-iron offset from the centre of the swept box per axis (needs 20 uT of span on every axis). |
| `altitude.rs` | `CMAltimeter`, `CMAltitudeData`, `CMAbsoluteAltitudeData`, `CMAmbientPressureData` | `absolute_altitude_m(p, p0)` is the standard atmosphere, `h = 44330 (1 - (p/p0)^(1/5.255))`. `RelativeAltimeter` zeroes at its first accepted pressure reading and reports the change in metres. Readings outside 30 to 110 kPa are refused. |
| `pedometer.rs` | `CMPedometer`, `CMPedometerData` (steps and distance) | `StepDetector` counts rising peaks of the user-acceleration magnitude over a threshold (0.12 g) with a 0.25 s refractory interval and re-arming below 60 % of the threshold. `Pedometer` keeps step times, reports distance as steps times stride (0.2 to 2.5 m, default 0.75 m), and counts steps in a time range. |
| `motion_activity.rs` | `CMMotionActivity` (stationary, walking, running, automotive, unknown, confidence) | `classify(magnitudes, window_s, steps)` labels a window from the spread of the magnitude and the step cadence. Thresholds: spread under 0.02 g is stationary; cadence 2.5 Hz or more is running; 1.0 Hz or more is walking; otherwise spread under 0.2 g is automotive. Confidence is low under 5 s, medium under 10 s, high from 10 s. |
| `rumble.rs` | `GCDeviceHaptics` (rumble part only), force-feedback rumble magnitudes | `Rumble` holds a strong and a weak motor level (clamped to 0..1, not finite is silent), a duration with `INFINITE_MS` for until stopped, and `magnitudes()` returns the two 16-bit motor values (0..65535). |
| `gamepad.rs` (extended) | `FFDeviceGetForceFeedbackCapabilities`, the capability part of the force-feedback device | `supports_rumble(force_feedback_bitmap)` checks the rumble effect bit (code 0x50) of the force-feedback event bitmap. |
| `lib.rs` | | Five `pub mod` lines for the new modules. |

Each new module has a `#[cfg(test)] mod tests` with at least five behaviour tests (the magnetometer has six, the
altitude five, the pedometer six, the activity six, the rumble five; the gamepad test is one added to the existing
module's tests).

No build or test was run in this round, as the task required. The code was checked by reading it. The numbers in the
tests were worked out by hand (the textbook 1000 m point, the 110 m climb from 101.325 to 100 kPa, the tilted-device
heading with the top edge 30 degrees up). The lead should run `cargo test -p bifrost-core` (or the crate's name in
`springboard/core/Cargo.toml`) before merging.

## Not built, and why

- No SDK API and no sensor reader. The shell reads the accelerometer only for screen rotation
  (`springboard/src/shell/rotate.rs`). Raw accelerometer and gyroscope events, the device-motion stream and the
  sensor recorder stay gaps: they need a sensor source and an app-facing surface, and this round has no hardware
  access.
- The rumble model has no locality (which actuator). The official page for the haptics engine lists localities, but
  this round did not fetch the locality enumeration, so no names were invented.
- True north needs a declination model. The module takes the declination as an input and does not supply one.
- The attitude reference frame (magnetic north, true north, arbitrary) is not wired to the existing estimator in
  `springboard/core/src/motion.rs`. The official reference-frame page was read through DocC (see below) but not
  recorded with REA.
- The recorded-data and batched-history row is not closed. `springboard/core/src/sensor_log.rs` already holds a
  time-ordered log, but nothing records sensor samples into it.

## Official API names mirrored

- `CMAltimeter` (altitude availability, relative and absolute altitude updates)
- `CMMagnetometerData` (field in microtesla)
- `CMPedometer` (step counting, distance, time-range queries)
- `CMMotionActivity` (activity kinds and confidence; the official flags are not exclusive, the Bifrost label is one
  value)
- `GCDeviceHaptics` (rumble levels only)
- `CMAttitudeReferenceFrame` (read through DocC, not used by the code)

## REA evidence (official documentation pages, rendered)

| Page | REA evidence id |
|---|---|
| developer.apple.com/documentation/coremotion/cmaltimeter | `ev_00c83c38b23ed2fb3d0373f94d52cbd4e115c60b61e5302dc889af6f3409fd31` |
| developer.apple.com/documentation/coremotion/cmmagnetometerdata | `ev_c6bcd845fbd925f84a7f81f9c26f78c1e9144712acb4177b95417a8584da00b3` |
| developer.apple.com/documentation/coremotion/cmpedometer | `ev_b3d6d192e8d911693e3a55ab9a2fc6b21d78fcb8a581aea77539f18abdf29681` |
| developer.apple.com/documentation/coremotion/cmmotionactivity | `ev_878743a3967ead6baa41139c54adad2bd92c267b07997a53c0e9257c60a83476` |
| developer.apple.com/documentation/gamecontroller/gcdevicehaptics | `ev_f29cf79ca4c58a27a7a49db32674f791b8d7ec48cdda92db1e96b49182e0c32e` |

Not recorded with REA: the force-feedback capability and the core-location heading pages. The force-feedback row is
built from the evdev bitmap convention, not from an official page, and the heading row cites the compass concept only.

## Matrix rows closed

Status is `partial` for every row: a model exists, but no app-facing API or sensor feed does.

| Slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-motion | Magnetometer | partial | `springboard/core/src/magnetometer.rs` |
| core-motion | Altitude and ambient pressure | partial | `springboard/core/src/altitude.rs` |
| core-motion | Pedometer, step counter and odometer | partial | `springboard/core/src/pedometer.rs` |
| core-motion | Motion activity (walking, running, stationary, automotive) | partial | `springboard/core/src/motion_activity.rs` |
| game-controller | Haptics and rumble | partial | `springboard/core/src/rumble.rs` |
| force-feedback | Device discovery and capabilities | partial | `springboard/core/src/gamepad.rs` (`supports_rumble`) |
| core-location | Compass headings and body identification | partial | `springboard/core/src/magnetometer.rs` (`magnetic_heading`, `true_heading`) |

Rows the theme lists that stay open: `core-motion` Raw accelerometer events, Raw gyroscope events, Device motion
(attitude, rotation rate), Recorded sensor data and batched history; `game-controller` Motion sensing (gyro and
accelerometer), Input events, Button, axis, switch and direction-pad elements, Racing wheels, Virtual on-screen
controller; `force-feedback` Effect creation, download and playback, Device properties and force-feedback state;
`hiddriverkit` Stylus and pen input; `inputmethodkit` Per-session input controller.

## Tooling note

`python3 tools/parity/parity.py search "<capability>"` failed in this worktree with a `BadZipFile` CRC error while
loading the embedding cache (`tools/parity/semantic.py`, `Embedder.__init__`). The search was skipped; the code
search of the index found the existing `core/src/motion.rs`, `core/src/sensor_log.rs` and `core/src/gamepad.rs`
modules, which this round extends. The cache file needs regenerating by the lead.
