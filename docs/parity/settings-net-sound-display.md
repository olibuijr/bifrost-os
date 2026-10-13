# Settings: Wi-Fi, Bluetooth, Sound and Displays (round 75)

Slice owner: settings-net-sound-display. Work was code-only (no build run); the lead builds and checks on Midget.

Evidence: the shipped settings extensions for Wi-Fi, Bluetooth, Sound and Displays were not yet extracted under
`~/Projects/macos-ref/extract/ref27/` when this slice ran, so the work came from the existing shell code
(`springboard/src/shell/pages.rs`, `settings.rs`, `sound_page.rs`, `services/display_services.rs`) and the
backends (`bare/bluetooth.rs`, the Wi-Fi command set in `shell/mod.rs`).

## Per area

| Area | State before | State after this slice |
| --- | --- | --- |
| Wi-Fi list, join, Other…, Known Networks, details, auto-join, priority, forget | Wired to `WifiAction` and `bin/shell.rs` (netd command set) | Unchanged. Already usable. |
| Bluetooth pair, connect, disconnect, forget, use for sound output | Wired to `bare::bluetooth::Cmd` and `Effect::BluetoothOutput` | Unchanged. Already usable. |
| Bluetooth detail › Services | Stub: "None listed" (`services::bluetooth_service::SERVICE_NOTE`) | Still a stub. Needs service browsing in the bluetooth backend. |
| Sound output / input device pick, input level | `sound_page.rs` over `SoundView` from audiod | Unchanged. |
| Sound alert sound | Missing | Added: `Settings > Sounds > Alert Sound` (four built-in names, `options::ALERT_SOUNDS`), stored as the `alert_sound` option in `settings.ini`. Picking it persists the choice. Playback is not wired: the shell has no alert playback path yet (Lock Sound is also only a stored flag). |
| Displays resolution / refresh rate | Read-only (`display_mode`) | Unchanged. `display_services::set_mode` answers `pending` because the compositor owns the output mode. Needs an output-configuration path in wm before a picker can apply a mode. |
| Displays scale | Interface Scale page exists | Unchanged. |
| Displays arrangement | Not applicable | One output in the shell; no arrangement page. |
| Displays night shift | Night Shift page (`night_shift.rs`) | Unchanged. |

## Remaining (not closed here)

- Display mode picker and apply: needs the compositor output-configuration path (wm).
- Alert sound playback: needs an audiod playback call for the chosen tone.
- Bluetooth service browsing for the detail page.
- Midget check: the Sounds page with the new ALERT SOUND group and the checkmark on the chosen tone.
