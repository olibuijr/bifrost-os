# Wire 6: Game Controllers page, battery and button remapping

Wires the game controller models and the `gamepad` service (`docs/parity/wire-svc-gamepad.md`) into a Settings page.
Matrix rows of the Game Controller framework in `docs/parity-matrix/data/game-controller.json` are not edited here; a row
is `parity` only after the Midget test below.

## What is now wired

| Feature | BifrOSt path | What a user gets |
|---|---|---|
| Controller discovery and hot-plug | `springboard/src/shell/services/gamepad_service.rs` (`views`, `snapshot`), `springboard/src/shell/game_controllers_page.rs` (`spec`, list page) | Settings > General > Game Controllers lists each connected controller by name, with its battery. The list follows plug and unplug within one scan (0.5 s). |
| Battery and light on controllers | `gamepad_service.rs` (`read_battery`), `game_controllers_page.rs` (`battery_text`) | The battery percent and charging state of a wireless pad, read from its `power_supply` entry in sysfs. A wired pad shows no battery. The light is not wired. |
| Input events (polling and callbacks) | `gamepad_service.rs` (`tick`, `remap_change`) | Apps still receive `input` events; the button in each event is the remapped output. |
| Button, axis and direction-pad elements | `game_controllers_page.rs` (`Remap`, `Edit`, `apply`), `game_controllers_page.rs` (`controller_spec`, `live_rows`) | A per-controller button remap (Settings > General > Game Controllers > a controller > Buttons). The test view shows the physical buttons, both sticks, the triggers and the direction pad, live. |
| Controller remapping (not a matrix row, the Settings page's own feature) | `game_controllers_page.rs` (`Remap`, `load`, `pref_key`) | Remaps are saved in `settings.ini` under `gamepad_remap.<controller name>`. |

Not wired: haptics and rumble (`bifrost_core::rumble`), motion (`GCMotion`), the racing wheel, the virtual on-screen
controller, the light, and the web Gamepad API. See the remaining gaps.

## How a user reaches it

Settings > General > Game Controllers (the row sits after Login Items). The page lists the connected controllers.
Choose one to open its page:

- Battery: the charge level, or Not Available.
- Test: Pressed (the physical buttons, by name), Left Stick and Right Stick (x, y to two decimals), Triggers (left and
  right) and Direction Pad (a compass point). These rows update while the page is open, and show the controller as
  it sends, before remapping.
- Buttons: one row per physical button with its current output. Choosing a row moves that button to the next output in
  turn; if another button already sends that output, the two swap. Restore Default Buttons clears the controller's remap.

A remap saves at once and applies to every app that receives controller input.

## How it works

- The tick of the gamepad service publishes a snapshot (`gamepad_service::snapshot`) after each scan and each input
  change. A generation number moves on each change; the Settings tick checks it (`game_controllers_page::take_change`)
  and refreshes the open test rows in place with `ListPage::set_value`, so the scroll position stays.
- The remap is a one-to-one table over the thirteen buttons (`Remap`). `assign` swaps when the target is taken, so
  two buttons never send one output. A damaged saved text is read as identity.
- Remaps are keyed by controller name, because controller ids change between sessions. Two identical pads share a
  remap; names are lower-cased with non-letters turned into `_`.
- Input goes to the focused SDK app only, as before (`focused_app`).

## How to verify on the test PC (Midget)

Midget is the test machine (no VM). Order: host tests, then `bifrost pc midget ota` until `phase=accepted`, then a
screenshot.

1. Settings > General: the row Game Controllers is there. With no pad, the page says No Controllers.
2. Plug a USB pad, or pair a Bluetooth one (a wireless pad shows its battery percent). The page lists it within a second,
   with its name. Unplug it: it disappears.
3. Open the controller's page. Press South and move the left stick: the Pressed row says South, and the stick row changes
   between -1 and 1. The page refreshes without leaving it.
4. Choose the South row under Buttons: its value becomes East, and the East row shows South. Open a SDK app that
   prints `input` events (`bifrost_app::gamepad_service::event`): pressing the physical South button now reports
   `name: east`.
5. Restore Default Buttons: South reports `south` again. Reboot: the remap is still there (settings.ini).
6. Screenshot the page in Dark and Light (`bifrost pc midget appearance dark|light`).

Host tests: `springboard/src/shell/game_controllers_page.rs` (`tests`: cycle and swap, one-to-one after many cycles,
text round trip, damaged text, pref key, saved edit, battery and d-pad names, list and detail rows, live rows) and
`springboard/src/shell/services/gamepad_service.rs` (`tests`: remap applies to button changes only, battery read from a
HID device's `power_supply`, views carry name, live state and saved remap, plus the earlier scan, queue and `list`/`state`
tests). Not run in this pass; the lead builds once.

## REA evidence

- `ev_1cb8b96092fab33316fbd8b6925885bfa57c833568fcd50e2ef78d70181899c6`: rendered page of
  https://developer.apple.com/documentation/gamecontroller/gcdevicebattery (battery level and state, `GCDeviceBattery.State`).
- `ev_e95ac4f9bd89cdd107a54c3c50cd3fab1f9f44ef36af07f56042c7470836a3c6`: rendered page of
  https://developer.apple.com/documentation/gamecontroller/gcphysicalinputprofile (elements by name, `valueDidChangeHandler`,
  `lastEventTimestamp`, buttons, axes and d-pads).
- `ev_329de65316e445e11cf1dfb8c15b990d3f42e5b9412b7f7d6ba80ae5fdf94bfc`: rendered `gccontroller` page (from `wire-svc-gamepad.md`).

Official outline used: `python3 -I tools/parity/docc.py gamecontroller/gcdevicebattery --depth 0` and
`gamecontroller/gcphysicalinputprofile --depth 0`.

The official framework has no remapping API: a player remaps controllers in the system settings. The remap here is a
BifrOSt setting, and the matrix row for it is the element row below.

## Matrix rows

| slug | feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| game-controller | Controller discovery and hot-plug | `springboard/src/shell/services/gamepad_service.rs`, `springboard/src/shell/game_controllers_page.rs` |
| game-controller | Input events (polling and callbacks) | `springboard/src/shell/services/gamepad_service.rs` (`tick`, `remap_change`) |
| game-controller | Button, axis and direction-pad elements | `springboard/src/shell/game_controllers_page.rs` (`Remap`, `controller_spec`) |
| game-controller | Battery and light on controllers | `springboard/src/shell/services/gamepad_service.rs` (`read_battery`); battery only, light still a gap |

## Remaining gaps

- Battery is read once per scan (0.5 s) and only from a `power_supply` entry in the HID device's sysfs tree; a pad
  that reports its battery elsewhere shows Not Available.
- The light, haptics and rumble, motion, the racing wheel and the virtual controller are not wired.
- Remaps are per name, not per physical device path. Two pads with the same name share one remap.
- The test rows report the physical controller; the app sees the remapped output (by design, so a test shows what the
  pad sends).
- The axis travel is still the assumed standard range (see `wire-svc-gamepad.md`), so a stick may read slightly off.
- The Home button and the controller's system overlay are not handled by the page.
- Web Gamepad API in the browser engine is still unverified.
