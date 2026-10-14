# Parity sdk-os-06: system capability models for apps

Rows of the OS layer (`docs/parity-matrix/data/` and `os/`) whose capability was handled only by the system. Each row
gets an app-facing model in `springboard/ui/src/os_services/` (re-exported as `bifrost_app::os_services`). The models
are pure: they take values in and return decisions, so the system service keeps the state and performs the action.

| Module | Mirrors | Wired to the system | Still needs |
|---|---|---|---|
| `usb.rs` | Device and interface class codes, identifiers, matching (USB class definitions) | None; the shell finds input and camera devices by capability in `bare/input.rs` and `media/v4l2.rs` | Live device descriptors reaching apps |
| `time_limits.rs` | Daily limits, Downtime window, always-allowed apps, ignore grants | The shell policy in `shell/screentime/config.rs` has the same rule | The shell to use this model; the shield UI stays in the shell |
| `usage_record.rs` | One `key=value` day file per day, ranking, seven-day window | The shell tracker writes the same keys | An app-facing read API, if one is wanted |
| `drawable.rs` | Drawable pixel size, two or three drawables, swap-set memory | None; the presenter in the system layer owns buffers | A presenter that reads this model, and app display links |
| `idle.rs` | Auto-lock with Low Power shortening, system sleep preference | Constants match `shell/options.rs`, `shell/low_battery.rs`, `bare/sleep.rs` | An idle signal from the shell; the suspend decision stays in the system layer |
| `output_route.rs` | Speaker and headphones, jack-driven routes, headphone volume cap | Constants match `media/route.rs` | Driving the mixer from the model; Bluetooth and network outputs |
| `alarm.rs` | Weekday masks, grace window, next occurrence | None yet; `docs/alarm-service-design.md` describes the service | The alarm scheduler service and RTC wake |
| `permissions.rs` | Manifest permission names, unknown-name rejection, require check | Names match `bare/app_policy.rs` | A run-time query service; the shell still enforces at launch |

Tests: `springboard/ui/tests/parity_sdk-os-06.rs` (headless, no device, clock or display).

Not built: the render server row (the compositor is system-owned, and `render/src/motion.rs` already holds the shared
motion contract) and the JavaScript execution environment row (the Boa engine is internal to the browser; an app-facing
context would need a product decision and a new SDK dependency).
