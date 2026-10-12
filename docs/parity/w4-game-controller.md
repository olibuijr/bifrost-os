# Game Controller: wave 4 (parity/w4-game-controller)

Three gap rows of the Game Controller framework are built as pure Rust models in `springboard/ui/src/` (bifrost-ui).
None of them is wired into the shell or into apps yet, and none has run on the x86 VM or on Midget. They are host-tested
only, so the rows move to `partial`, not `parity`.

## What was built

- `springboard/ui/src/virtual_controller.rs`: a software pad laid over a touch surface. The layout is a list of controls
  (buttons, the left and right sticks, the direction pad) in unit-square coordinates. Touches are tracked by id. A button
  is pressed while a finger is on it. A stick or the direction pad reports the finger's offset from the control centre,
  divided by the radius and clamped to the unit circle. App-set values (`set_value`, `set_position`) apply while no finger
  is on that control. Drawing the pad is left to the caller.
- `springboard/ui/src/controller_motion.rs`: a motion profile fed from raw accelerometer (g) and gyroscope (rad/s)
  samples. A first-order low-pass filter (`alpha`, clamped to 0..=1) splits total acceleration into gravity and user
  acceleration. Attitude is the roll and pitch of the gravity vector. Yaw stays at zero, because gravity cannot give it;
  fusion with the rotation rate is not built.
- `springboard/ui/src/racing_wheel.rs`: a racing wheel model fed from evdev records. The steering axis (`ABS_WHEEL`) is
  centred and maps to -1..=1. The throttle (`ABS_GAS`) and brake (`ABS_BRAKE`) map to 0..=1. A sequential shifter
  (`BTN_GEAR_UP`, `BTN_GEAR_DOWN`) moves one gear per press, from neutral (0) up to the top gear, and ignores releases
  and repeats. Paddles and force feedback are not modelled.

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests`
with hand-worked expected values (virtual controller 6 tests, motion 7 tests, racing wheel 7 tests).

## Official API names mirrored

- Virtual controller: `GCVirtualController`, `init(configuration:)`, `updateConfiguration(forElement:configuration:)`,
  `connect(replyHandler:)`, `disconnect()`, `setPosition(_:forDirectionPadElement:)`, `setValue(_:forButtonElement:)`.
  Source: https://developer.apple.com/documentation/gamecontroller/gcvirtualcontroller
- Motion: `GCMotion`, `hasAttitude`, `hasRotationRate`, `hasGravityAndUserAcceleration`, `attitude`, `rotationRate`,
  `GCEulerAngles`, `acceleration`, `gravity`, `userAcceleration`, `sensorsActive`.
  Source: https://developer.apple.com/documentation/gamecontroller/gcmotion
- Racing wheel: `GCRacingWheel`, `connectedRacingWheels`, `acquireDevice()`, `relinquishDevice()`, `isAcquired`,
  `wheelInput`, `capture()`, `isSnapshot`.
  Source: https://developer.apple.com/documentation/gamecontroller/gcracingwheel

Not mirrored: `GCQuaternion` (attitude is Euler angles only), `GCVirtualController.connect`/`disconnect` (no screen or
device object), `sensorsActive` and `sensorsRequireManualActivation` (no sensor device), and the racing wheel's
discovery notifications and snapshots.

## REA evidence

- GCMotion page: `ev_77093bb21101f3766959520d5530f622300d15fe2219996fc53b142d538d816b`
- GCVirtualController page: `ev_699186b21b5e4078759d3b95719bca558cf272b41e4de702c92ef7fd6fc6c65e`
- GCRacingWheel page: `ev_12b66279d3e7b73f726b26196ba9fd2f3a95390484122b95b52b9d879ffbb76d`

The DocC JSON was read with `python3 -I tools/parity/docc.py gamecontroller/<symbol> --depth 2`.

## Matrix rows closed

Status moves to `partial` in the rows below. The matrix (`docs/parity-matrix/`) is updated centrally by the lead.

| Framework | Feature (as written in data/game-controller.json) | New status | BifrOSt path |
|---|---|---|---|
| Game Controller | Virtual on-screen controller | partial | springboard/ui/src/virtual_controller.rs |
| Game Controller | Motion sensing (gyro and accelerometer) | partial | springboard/ui/src/controller_motion.rs |
| Game Controller | Racing wheels, paddles and gear shifters | partial | springboard/ui/src/racing_wheel.rs |

## Limits and next steps

- Nothing is wired to the input decoder (`springboard/src/bare/input.rs`) or to the event loop. Raw gamepad and wheel
  records must be routed to these models by the shell before any app sees them.
- No app-facing SDK API exists for these models yet. The `bifrost_app` SDK does not re-export them.
- Racing wheel paddles and force feedback are missing, so the row stays partial.
- Motion has no yaw, no quaternion and no sensor activation state.
- The Linux codes used for the wheel (`ABS_WHEEL` 0x08, `ABS_GAS` 0x09, `ABS_BRAKE` 0x0a, `BTN_GEAR_DOWN` 0x150,
  `BTN_GEAR_UP` 0x151) are taken from the kernel input event codes and must be checked on a real wheel.
- Not built here (rows left as they were): input events, the Home button and controller settings page, and the
  keyboard and mouse profile.
- Tests have not been run: this round had no builds, by rule. The first integration build must run
  `cargo test -p bifrost-ui` for the three modules.
