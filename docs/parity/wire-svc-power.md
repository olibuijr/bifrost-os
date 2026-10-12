# Wire: the `power` system service (ProcessInfo power state and battery)

Status: wired in the code on branch `wire/svc-power`, not yet built or run on the test PC. Not promoted in the parity
matrix (the rows stay as they are until the lead builds and a Midget run passes).

## What is now wired

| Matrix file (`data/<file>.json`) | Feature (as the row names it) | BifrOSt path | Now |
|---|---|---|---|
| `foundation-networking-lowlevel` | Processes and Threads: process information and activity (ProcessInfo) | `springboard/sdk/src/process_info.rs`, `springboard/sdk/src/power_service.rs`, `springboard/src/shell/services/power_service.rs` | Apps get the thermal state and Low Power Mode through the `power` service too. Change events (`thermalStateDidChange`, `powerStateDidChange`) are pushed to every running app. |
| `iokit` | Thermal state (surfaced through ProcessInfo.thermalState) | `springboard/core/src/thermal.rs` | The model moved from the SDK into `bifrost_core::thermal` so the shell can reuse it (no duplicate). The SDK re-exports the same types. Now also a change event. |
| `iokit` | Power sources and battery state | `springboard/sdk/src/power_sources.rs`, `springboard/core/src/power_supply.rs` | Apps read the first battery through the service (`battery`, `batteryDidChange`). |

The rows are the ones in `docs/parity-matrix/data/` (the matrix itself is not edited here). The Task Management row in
`foundation-app-support` also names ProcessInfo activities and thermal state; it is not changed here.

### Methods (service `power`)

- `thermalState` -> `{"thermalState": "nominal"|"fair"|"serious"|"critical"}`
- `lowPowerMode` -> `{"lowPowerMode": bool}` (the shell's Low Power Mode setting)
- `battery` -> `{"battery": {"present", "percent", "state", "pluggedIn", "minutesToEmpty", "minutesToFull"}}`
- `snapshot` -> all three keys at once.

Events go to every running app (`app: None`), with the same keys:
`thermalStateDidChange`, `powerStateDidChange`, `batteryDidChange` (the battery event fires when the percent, charging
state or plug state changes, not on each energy reading). Sensors are read at most every 10 s.

### Other changes

- `springboard/core/src/thermal.rs` (new): the `ThermalState` model, `thermal_state_for` and `worst_in(class_dir)`.
  `springboard/core/src/lib.rs` gets one `pub mod thermal;` line.
- `springboard/sdk/src/process_info.rs`: the thermal model is now a re-export of `bifrost_core::thermal`; its one
  thermal test moved to `core/src/thermal.rs`. The public names (`ThermalState`, `TripPoint`, `thermal_state_for`,
  `ProcessInfo::thermal_state`) are unchanged.
- The shell writes Low Power Mode to `/run/bifrost/power-state` (`low_power=0|1`) whenever it changes, and once at the
  first tick. Before this, nothing wrote that file, so `ProcessInfo::is_low_power_mode_enabled()` always read false in
  apps. The shell path is a literal that must equal the SDK's `POWER_STATE_PATH`; both are tested.

## How a user reaches it

- Apps: `bifrost_app::power_service::{thermal_state, low_power_mode, battery, snapshot}` returns a request id; the reply
  comes to `App::service_reply` and is decoded with `decode_thermal_state`, `decode_low_power_mode`, `decode_battery` or
  `decode_snapshot`. Pushed changes come to `App::service_event` and decode with `decode_change`.
- Users: toggling Low Power Mode in the battery menu or Settings now reaches apps through the file and the event; the
  thermal state and battery are read from the machine.

## How to verify on the test PC (Midget)

1. Build and publish as usual, then `bifrost pc midget ota` until `phase=accepted`, and take a screenshot.
2. On Midget: `cat /run/bifrost/power-state` shows `low_power=0`. Turn Low Power Mode on in Settings and the file shows
   `low_power=1`.
3. `cat /sys/class/thermal/thermal_zone*/temp` and `cat /sys/class/power_supply/*/capacity` give the raw values the
   service reports (the device with no battery answers `{"battery":{"present":false}}`).
4. Host tests: `cargo test -p bifrost-core thermal` and the `power_service` tests in springboard and bifrost-app (run by
   the lead once, not by this round).

## REA evidence (official documentation, rendered page)

- `ev_c0e440ec671217bfb8b322772fcdbed11e815692b3732ffab5211bba4f33660c`: ProcessInfo overview
  (`developer.apple.com/documentation/foundation/processinfo`), the thermal-state and low-power sections.
- `ev_2523ebb2c0a83d1eb6b91c2e0ff718d288a8aa6b105137caa7cde195f49957a5`: `ProcessInfo.ThermalState`.
- `ev_363ee262aa13879378ba1bc0527160de81ff0bf368cf2fb5ad6228618a0ce238`: `isLowPowerModeEnabled`.

The DocC symbol facts came from `python3 -I tools/parity/docc.py foundation/processinfo --depth 1` (thermalState,
isLowPowerModeEnabled, thermalStateDidChangeNotification, the power-state notification).

## Remaining gaps

- Per-process energy impact (the MetricKit row for energy stays Gap): there is no model and the shell does not measure
  per-process CPU or GPU energy. Not answered by the service.
- Only the first battery is reported; machines with several supplies show the first one (`IOPowerSources` lists them all).
- Change events are broadcast to every running app; there is no per-app subscription, so each app filters by event name.
- Thermal state is the worst zone from sysfs with the kernel trip points (or fixed 75/85/95 C limits when a zone has no
  trips). There is no finer per-zone detail and no kernel notification, only the 10 s poll.
- No shell UI was changed: the battery menu and Settings still show what they showed before.
