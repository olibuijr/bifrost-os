# Game Controller parity

Official reference: https://developer.apple.com/documentation/gamecontroller (landing page, its "Discovering game
controllers", "Handling input events" and "Input" topics, and the element and profile symbols listed there).
Matrix row: `docs/parity-matrix/data/game-controller.json`.

## What was built

Two pure std modules, no new crates, no `unsafe`, no hardware and no network:

- `springboard/core/src/gamepad.rs` (`bifrost_core::gamepad`): controller discovery. `Capabilities` reads the
  evdev key and absolute-axis bitmaps and decides `is_game_controller()`: a face button plus both left-stick axes,
  and no `BTN_LEFT` or `BTN_TOUCH`. `Registry` keeps the connected controllers, assigns ids that are never reused, and
  returns one `Event::Connected` or `Event::Disconnected` per device, for paths it has not seen or still holds.
- `springboard/ui/src/gamepad.rs` (`bifrost_ui::gamepad`): the input model. `element(ty, code)` maps raw evdev records
  to `Button` (13 digital elements), `Axis` (two sticks, two triggers) and the direction pad. `AxisRange` normalises a
  raw value: a centred stick maps `min` to -1 and `max` to 1, a trigger maps to 0..=1, and a dead zone is removed and
  the rest rescaled so there is no step at its edge. `Gamepad::feed` returns a `Change` only when a button edge, a
  normalised axis value or a direction-pad component moves, and ignores key repeats (value 2).

Both modules are registered with one `pub mod gamepad;` line each (`springboard/ui/src/lib.rs`,
`springboard/core/src/lib.rs`).

## Official names this mirrors

| Official symbol or topic | BifrOSt name |
|---|---|
| GCController, controllers(), controllerDidConnectNotification | `bifrost_core::gamepad::Registry`, `Event::Connected` |
| GCDevice (a device with input) | `Controller` |
| Discovering game controllers | `Capabilities::is_game_controller` |
| GCDevicePhysicalInput, valueChangedHandler | `Gamepad::feed`, `Change` |
| GCButtonElement and the button element names | `Button` |
| GCAxisElement | `Axis`, `AxisRange` |
| GCDirectionPadElement | `Change::Dpad`, `Gamepad::dpad` |
| GCPhysicalInputExtents (scale to physical units) | `AxisRange::min`, `AxisRange::max` (device travel, no physical units) |

The element names follow the Linux gamepad codes that every standard pad reports, which is what the kernel exposes
on the evdev node. The Apple element names are documented here only; the code uses BifrOSt names.

## REA evidence

- `ev_ff53898eca6b648f970bb4815e6180d58c7ae0c2ca3afd4259bf68122bd86030`: rendered landing page of
  https://developer.apple.com/documentation/gamecontroller (captured in this pass with `rea-page.sh`).
- `ev_bde24e23f467e2d68b90e844d442131976f037dab9c6aee769ca836cfdcb6436`: the earlier capture recorded in the matrix row.

The official outline (`docc.py gamecontroller --depth 2`) was used for the topic list and the element symbols.

## Tests

Each module has its own `#[cfg(test)] mod tests`: 6 behaviour tests in `springboard/ui/src/gamepad.rs` (element
mapping, button edges and repeats, centred stick with dead-zone rescaling, unipolar trigger, direction-pad changes,
axis change suppression) and 6 in `springboard/core/src/gamepad.rs` (device classification, bitmap bit order,
connect ids and duplicates, non-controllers taking no id, disconnect and id reuse, arrival order and lookup).
These tests were written but not run in this pass: the stage that builds and tests the repo runs them.

## Remaining gaps

- Not wired to the live path. `springboard/src/bare/input.rs` still treats ABS_X and ABS_Y as a touchscreen pointer
  and routes no gamepad codes to `bifrost_ui::gamepad`. The hot-plug watch in
  `springboard/src/bare/chord_recovery.rs` does not feed `bifrost_core::gamepad::Registry`.
- No app-facing SDK API. Apps cannot list controllers, receive connect events or read state through `bifrost_app`.
- No haptics or rumble (`GCDeviceHaptics`). `springboard/src/bare/vibrator.rs` only checks for a vibrator node.
  The rumble effect encoding (the kernel's force-feedback structure) is not written.
- No battery, light (`GCDeviceLight`) or motion (`GCMotion`) data.
- No virtual controller (`GCVirtualController`) for touch-only devices.
- No switch element, element collection or physical input source (remapping, `GCPhysicalInputSource`).
- No racing wheel model (paddles, gear shifter, steering wheel) and no force feedback.
- No controller Home button settings and no controller page in Settings.
- Web Gamepad API status is unverified: the browser engine enables Servo's gamepad feature, but whether it receives
  evdev controller events was not checked.

## Keyboard, mouse and web gamepad input (sdk-rest-09)

- `springboard/ui/src/game_input.rs`: `KeyboardProfile` and `MouseProfile` turn evdev key and relative records into
  edges, a held set, per-frame motion and wheel steps, and a release on focus loss. Tested headless in
  `springboard/ui/tests/parity_sdk-rest-09.rs`.
- `springboard/ui/src/web_gamepad.rs`: `snapshot` lays a controller state out in the web standard mapping (17 buttons,
  4 axes). Tested headless in the same file.
- Needs the lab machine: the shell event loop does not feed the keyboard and mouse models yet, and the browser engine
  does not receive the gamepad snapshot. Confirm both on Midget with a real keyboard, mouse and controller.
