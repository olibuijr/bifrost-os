# Wire: the `gamepad` system service

Wires the game controller models (`bifrost_core::gamepad`, `bifrost_ui::gamepad`) into the desktop and gives apps a
typed client. Matrix rows: the Game Controller framework in `docs/parity-matrix/data/game-controller.json` (not edited
here; the rows are promoted only after the Midget test).

## What is now wired

| Feature (matrix row feature) | BifrOSt path | Status after this change |
|---|---|---|
| Controller discovery and hot-plug | `springboard/src/shell/services/gamepad_service.rs` (`scan`) | Wired: sysfs scan of `/sys/class/input/eventN` every 0.5 s, `Registry` from `bifrost_core::gamepad` |
| Input events (polling and callbacks) | `gamepad_service.rs` (`drain`, `change_json`), `springboard/sdk/src/gamepad_service.rs` (`event`) | Wired: one `input` event per button edge, normalised axis change or direction-pad change, to the focused app |
| Button, axis and direction-pad elements | `gamepad_service.rs` (`BUTTON_NAMES`, `AXIS_NAMES`) | Wired by name on the wire (`south`, `leftX`, ...) |
| Battery and light, motion, racing wheel, virtual controller, rumble | not wired | Gap (see below) |

Connect and disconnect (`connected`, `disconnected`) go to every running app. Input goes to the focused app only:
the shell's front window, when it is an installed SDK app (`AppId::External`).

## Wire surface (service channel, no protocol change)

Service name `gamepad`, called through `bifrost_app::service::call`:

- `list` returns `[{"id", "name", "path"}]`, the connected controllers in arrival order.
- `state` with `{"id": n}` returns `{"id", "buttons": [names], "axes": {"leftX", "leftY", "rightX", "rightY",
  "leftTrigger", "rightTrigger"}, "dpad": {"x", "y"}}`. An unknown id is an error reply.

Events pushed through `App::service_event`:

- `connected` and `disconnected`: a controller object, to every running app.
- `input`: `{"id", "kind": "button", "name", "pressed"}`, `{"id", "kind": "axis", "name", "value"}` or
  `{"id", "kind": "dpad", "x", "y"}`, to the focused app.

Button names: `south`, `east`, `north`, `west`, `leftShoulder`, `rightShoulder`, `leftTrigger`, `rightTrigger`,
`leftStick`, `rightStick`, `select`, `start`, `home`. Axis names: `leftX`, `leftY`, `rightX`, `rightY`, `leftTrigger`,
`rightTrigger`.

## How an app reaches it

SDK client `bifrost_app::gamepad_service` (`springboard/sdk/src/gamepad_service.rs`):

- `list(ctx)` and `state(ctx, id)` return a request id; the reply arrives in `App::service_reply` and is decoded with
  `parse_list` or `parse_state`.
- `event(service, event, body)` decodes an `App::service_event` into `Event::Connected`, `Event::Disconnected` or
  `Event::Input { controller, change }`, where `change` is a `bifrost_ui::gamepad::Change`. It returns `None` for
  other services, so an app can pass every service event through it.

The SDK reads the name tables from `springboard::shell::services::gamepad_service`, so the two sides cannot drift.

## How a user reaches it

The service runs in the shell from boot; no setting turns it on. A user plugs in a game controller (USB or paired
Bluetooth HID). An installed app that uses the client receives the events while it is in front. There is no Settings
page and no menu item for controllers yet.

## How to verify on the test PC (Midget)

Midget is the test machine (owner rule: no VM). Order: host tests, then `bifrost pc midget ota` until
`phase=accepted`, then a screenshot.

1. Plug a controller into Midget. Check the node and its capabilities:
   `ssh midget 'grep -H . /sys/class/input/event*/device/name'` lists the controller's name; the pad node has `key`
   bits in the 0x130..0x13e range.
2. From an app that calls `gamepad_service::list`, the reply lists the pad with an id, and `connected` arrived when
   it was plugged in while the app ran.
3. Press a face button with the app in front: an `input` event with `kind: button`, `name: south`, `pressed: true`
   reaches the app; release gives `pressed: false`. Move a stick: `kind: axis` events with values between -1 and 1.
4. Switch the front window to a shell app (Settings): the app that was in front gets no further `input` events.
5. Unplug: `disconnected` arrives within about one scan interval (0.5 s); `list` no longer shows the pad.

Host tests: `springboard/src/shell/services/gamepad_service.rs` (`tests`: bitmap order, sysfs connect and disconnect,
queued records to changes, input JSON, `list` and `state` answers, names and focus) and
`springboard/sdk/src/gamepad_service.rs` (`tests`: event decoding, state and list parsing). Not run in this pass; the
lead builds once.

## REA evidence

- `ev_329de65316e445e11cf1dfb8c15b990d3f42e5b9412b7f7d6ba80ae5fdf94bfc`: rendered page of
  https://developer.apple.com/documentation/gamecontroller/gccontroller (controller discovery, `controllers()`,
  `GCControllerDidConnect` and `GCControllerDidDisconnect`, the profile and input-handler model). Captured in this pass.
- `ev_ff53898eca6b648f970bb4815e6180d58c7ae0c2ca3afd4259bf68122bd86030`: landing page (from `game-controller.md`).

Official outline used: `python3 -I tools/parity/docc.py gamecontroller/gccontroller --depth 1`.

## Design notes

- No protocol change: the service channel carries everything (`ToShell::ServiceCall`, `ToApp::ServiceEvent`).
- Discovery is sysfs-only (capability bitmaps are parsed from the hex words the kernel prints), so no `unsafe` and no
  ioctl. Reading the records is a plain `read_exact` of 24 bytes on a reader thread per controller; the thread ends
  when the node goes away.
- Axis ranges are the standard pad defaults (signed 16-bit sticks with a 10 % dead zone, triggers 0..255) because the
  device's real travel needs `EVIOCGABS`. A pad with another travel reads its sticks slightly off.
- Controller ids come from `bifrost_core::gamepad::Registry`: never reused in a session.
- The shell's bare input path (`springboard/src/bare/input.rs`) is untouched. It still maps `ABS_X`/`ABS_Y` as a
  touchscreen pointer; that is a separate step.

## Remaining gaps

- Ticks run with the shell's frame loop. If the shell sleeps while idle, input can arrive late; to check on Midget.
- Axis travel is assumed, not read from the device (see above).
- No haptics or rumble (`bifrost_core::rumble` is not connected to a force-feedback write).
- No battery, light, motion (`GCMotion`) or racing wheel input. No virtual controller.
- No `GCControllerLiveInput` snapshot API, no current-controller or player index, no Bluetooth pairing UI.
- No Settings page for controllers and no controller Home button handling.
- Web Gamepad API in the browser engine is still unverified.
