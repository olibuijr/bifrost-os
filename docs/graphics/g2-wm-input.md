# G2: input, pointer preferences and pointer protocols (`wm-input`)

Owner: wave G2 agent `wm-input`. Contract: `docs/graphics-architecture.md`, "Window server" (`input.rs`: libinput
configuration, gestures, key repeat, input straight to the focused surface). Code: `springboard/wm/src/input.rs`,
with three one-line hooks in `springboard/wm/src/state.rs` (keyboard, protocol globals, gesture state) and
`springboard/wm/src/drm.rs` (libinput `DeviceAdded`).

## What is implemented

### Preferences (`InputPrefs`, pure)

Read from the shell's `settings.ini` (`InputPrefs::load(&prefs_path())`). `prefs_path()` follows the shell's data
directory rule (root: `/var/lib/springboard`; a user: `$XDG_DATA_HOME/springboard` or `~/.local/share/springboard`).
`BIFROST_SETTINGS` overrides the file. Keys, defaults and clamps:

| Key | Values | Default |
|---|---|---|
| `pointer_accel` | `adaptive` or `flat` | `adaptive` |
| `pointer_speed` | -1.0 to 1.0 (NaN ignored) | 0.0 |
| `natural_scroll` | flag | off |
| `tap_to_click` | flag | on |
| `two_finger_secondary_click` | flag (clickfinger method) | on |
| `key_repeat_delay_ms` | 100 to 2000 | 300 |
| `key_repeat_rate` | 1 to 100 per second | 30 |
| `keyboard_layout` | two or three lowercase letters (`us`, `is`) | `us` |
| `cursor_size` | 16 to 96 px | 24 |
| `cursor_theme` | a directory name, no `/` or spaces | `default` |

Flags accept `1 on true yes` and `0 off false no`. A bad value keeps its default. A later line wins.

### libinput (`plan_device`, `configure_device`)

`plan_device` maps the prefs onto a device's capabilities (`DeviceCaps`: accel, natural scroll, tap, clickfinger).
A setting is written only where the device supports it, so a mouse gets the accel profile and speed, and a
touchpad also gets natural scroll, tap and the click method. `configure_device` (feature `drm`) reads the caps
from the libinput device, applies the plan with `config_*` calls and logs any refused setting. The drm loop calls
it on `DeviceAdded` with the prefs file read at that moment.

### Keyboard (`add_keyboard`)

The seat's keyboard takes the layout (`XkbConfig.layout`) and the repeat delay and rate from the prefs, so
`wl_keyboard.repeat_info` carries them. Before, the seat used the fixed values 300 ms and 30 per second, which are
the defaults now.

### Cursor (`xcursor_dirs`, `cursor_env`)

`xcursor_dirs` gives the search order for a theme (`~/.icons`, then `icons/` under each XDG data directory) and
`cursor_env` gives `XCURSOR_THEME` and `XCURSOR_SIZE` for clients. Nothing draws the cursor image yet (see todo).

### Pointer gestures (`zwp_pointer_gestures_v1`)

The backend's swipe, pinch and hold begin, update and end events are forwarded to the pointer focus with
Smithay's `PointerHandle::gesture_*`. `GestureGate` keeps each kind's begin and end paired: a begin while that kind
runs is dropped, an update or end without a begin is dropped, and `cancel_all` lists the running kinds for a focus
change. `delegate_pointer_gestures!` serves the global, created by `register_protocols`.

### Pointer constraints and relative pointer

- `zwp_relative_pointer_manager_v1`: every backend motion is also sent as relative motion (`delta` and
  unaccelerated `delta_unaccel`, microseconds) to the focused surface, even while the pointer is locked.
- `zwp_pointer_constraints_v1`: a constraint on the focused surface is activated on enter and deactivated on
  leave (`update_constraint_focus`). A new constraint on the focused surface activates at once. While a locked
  constraint is active the absolute pointer does not move (`motion_locked`). A `cursor_position_hint` from a
  locked client moves the pointer to the hint.

## Mapping to the contract

| Contract item | Where |
|---|---|
| libinput configuration (acceleration, natural scrolling, tap) | `plan_device`, `configure_device`, drm `DeviceAdded` hook |
| Gestures (pinch, swipe, hold) forwarded to the focused client | `process_input_event` gesture arms, `gesture_*` methods, `GestureGate` |
| Key repeat | `add_keyboard` (wl_keyboard.repeat_info) |
| Input straight to the focused surface | unchanged: `pointer_motion`, `pointer_button`, `key` |
| Skeleton signatures | kept: `process_input_event`, `pointer_location`, `pointer_motion`, `pointer_button`, `key` |

## Tests

Host tests in `input.rs` (`cargo test -p bifrost-wm input::` once the lead runs the suite; this agent ran only
`cargo check --tests`):

- defaults equal the old seat values and `from_lookup` with no keys
- flag spellings, number clamps and NaN, layout and theme validation
- `settings.ini` parsing (comments, spaces, last line wins)
- `plan_device` with full caps, and with a mouse (no natural scroll or tap written)
- xcursor search order and `XCURSOR_*` env
- `GestureGate` begin and end pairing, cancel on focus change
- `motion_locked` truth table

## Verification on Midget

Midget: Intel HD 530, Mesa ANV, Vulkan 1.4. The session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`. Steps (the shell has no pointer or trackpad page yet, so the prefs are written by hand):

1. Build and push the window server: `cargo build --offline --profile fast --bin bifrost-wm` in `springboard/`,
   then copy it to Midget (`tools/midget-push-shell.sh --no-build` is the shell path; the window server needs the
   same treatment).
2. On Midget, write `/var/lib/springboard/settings.ini` (or `~/.local/share/springboard/settings.ini` as the user)
   with `pointer_accel=flat`, `natural_scroll=1`, `key_repeat_rate=40`, `keyboard_layout=is`, then restart
   `bifrost-wm`. Check the log for `bifrost-wm: ...: natural scroll: ...` errors (none expected on a touchpad).
3. Scroll with the touchpad: content moves the other way. Hold a key: repeats at 40 per second.
4. Run a gesture client (for example `libinput debug-events` on Midget shows the gestures; a three-finger swipe
   reaches a client that binds `zwp_pointer_gestures_v1`).
5. Pointer lock: a game or `wp_pointer_constraints` test client locks the pointer; the cursor stays and relative
   motion keeps arriving (`WAYLAND_DEBUG=1` shows `locked` and `relative_motion`).
6. Input to photon: `tools/bench.py --midget` once `bench-graphics` lands.

## What is left (todo_left)

- The shell writes none of the pointer or trackpad keys yet. A Settings page for them (Trackpad, Mouse,
  Keyboard repeat, cursor size) is the shell's job; the keys above are the contract.
- The keyboard layout: the shell keeps the enabled mask and active layout in `kb_layouts.ini`, not in
  `settings.ini`. The window server reads `keyboard_layout`; the shell should write it when the layout changes, or
  the window server should read the shell's state.
- Repeat and layout changes at runtime: `add_keyboard` runs at start only. A change needs `change_repeat_info`
  (Smithay has it) on the keyboard handle and a re-read on a prefs change.
- Libinput settings change on a prefs write: only on `DeviceAdded` today. Re-applying to live devices needs the
  device list kept in `WmState`.
- Cursor image: `cursor_image` in `handlers.rs` is empty and nothing loads the xcursor theme. Loading the theme
  needs an xcursor parser (no new crates were allowed); the render server draws the image.
- Constraints: confined pointers are activated but not clamped to their region (the region is in
  `ConfinedPointer::region`). Locked-pointer deactivation on a session lock is not handled.
- Gesture recognition is forwarding only. Compositor-level gestures (four-finger desktop swipes) are not
  recognised here; they belong to the shell's `four_finger` setting and need the keybinding hook.
- Kinetic scrolling is not implemented.
- `register_protocols` drops the state objects; the globals stay for the display's lifetime, as Smithay documents
  for `new` (not verified on a running session).
- Not run: the host tests (only `cargo check --tests`), and nothing has run on Midget.
