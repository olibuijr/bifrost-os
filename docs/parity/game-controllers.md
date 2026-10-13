# Game controllers (round 75, slug game-controllers)

Official reference: https://developer.apple.com/documentation/gamecontroller and its topics "Discovering game
controllers", "Handling input events", "Responding to controller haptics" (rumble) and the Game Controllers settings
pane. Matrix rows: `docs/parity-matrix/data/game-controller.json` (features named below; the matrix is not edited here).

## Reference evidence

- `~/Projects/macos-ref/extract/ref27-index.tsv` lists `GameControllerMacSettings.appex` and
  `GameControllerSettingsIntents.appex` under `System/Library/ExtensionKit/Extensions/`, but the extracted
  `ref27/` tree has no GameController bundle yet (only `System` and `usr` roots). The settings pane was therefore
  built from the existing shell code and the controller docs, not from the extension's `Base.lproj` strings or nib.
  Re-check the pane's labels and layout against those files once the lead extracts them.
- Official outline and element names: `python3 -I tools/parity/docc.py gamecontroller --depth 2` (see
  `docs/parity/game-controller.md`, which records the REA captures).

## What was already built and is reused

- Discovery and hot-plug: `springboard/core/src/gamepad.rs` (`Registry`, `Capabilities`), wired in
  `springboard/src/shell/services/gamepad_service.rs` (sysfs scan, reader threads, `connected`/`disconnected`).
- Input model and events: `springboard/ui/src/gamepad.rs` (`Gamepad::feed`, `Change`), `input` events to the front app.
- Settings pane with live test rows, battery and per-controller remap: `springboard/src/shell/game_controllers_page.rs`.
- SDK client: `springboard/sdk/src/gamepad_service.rs` (`list`, `state`, `event`, `parse_list`, `parse_state`).

## What this round adds

1. Rumble on the device (`Haptics and rumble` row). `springboard/src/bare/pad_rumble.rs` (`Motor`) uploads one
   `FF_RUMBLE` effect to the controller's `eventN` node with `EVIOCSFF`, plays it with an `EV_FF` write and stops and
   erases it with `EVIOCRMFF`. The effect bytes and play event are pure functions with unit tests. A `Motor` is opened
   only for a node whose force-feedback bitmap has the rumble bit (`bifrost_core::gamepad::supports_rumble`), when the
   controller connects, and it is dropped when the controller goes; closing the node erases its effects.
2. Rumble through the service: `gamepad` method `rumble` with `{id, strong, weak, duration}`. Levels are 0 to 1,
   duration in milliseconds (default 500, 0 stops). Only the app in front may call it; any other caller gets an error.
   The SDK call is `bifrost_app::gamepad_service::rumble(ctx, id, strong, weak, duration_ms)`.
3. Home button. A physical Home press (the edge, before any remap) opens the launcher: in desktop mode
   `Shell::appgrid_show` (new in `springboard/src/shell/overview.rs`, it does not close an open launcher); otherwise the
   shell's home action (`assistive_action(Action::Home)`). Ignored while the screen is locked. The app in front still
   receives the Home input as before.
4. `front_app(shell)` is now one helper used by both input delivery and rumble, replacing the inline lookup in `tick`.

## Official names this mirrors

| Official symbol or topic | BifrOSt |
|---|---|
| GCDeviceHaptics, rumble motors (strong, weak) | `pad_rumble::Motor::play`, `bifrost_core::rumble::Rumble` |
| GCController.controllers(), controllerDidConnectNotification | `gamepad_service` `connected`/`disconnected` |
| GCDevicePhysicalInput, valueChangedHandler | `gamepad_service` `input` events |
| Home button (GCControllerButtonInput, system button) | `press_home` in `gamepad_service.rs` |

## Tests

Host unit tests were written, not run (no builds in this round):
- `springboard/src/bare/pad_rumble.rs`: effect bytes (magnitudes, length, cut at 65535 ms, zero length for until stopped),
  play event bytes, silent or zero-length rumble ends at once, missing node cannot open a motor.
- `springboard/src/shell/services/gamepad_service.rs`: rumble with no id and with no motor returns an error.
- `springboard/sdk/src/gamepad_service.rs`: `rumble_body` carries id, clamped levels and duration.

## Needs checking on Midget (not verified)

- A real rumble pad: the kernel accepts `EVIOCSFF` for the effect layout (48-byte `ff_effect` on 64-bit), the motor
  runs for the duration, and a stop erases it. The shell must be able to open the event node for writing.
- Home press opens the launcher in desktop mode and goes home in the phone layout, with a real pad.
- The Game Controllers pane's live test and remap still need a visual check against the extension's labels.

## Still open (matrix rows not closed here)

- Web Gamepad API in the browser engine: not verified.
- Keyboard and mouse as game input (`os` row): unchanged.
- Racing wheel force feedback: `force_effect.rs` is still not uploaded to nodes; only rumble is.
- SDK input profiles and `GCVirtualController` wiring: unchanged (pure models in `ui`).
