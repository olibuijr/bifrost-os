# Core Graphics system services parity (cg-services)

Scope: the system-services half of Core Graphics / Quartz (not the drawing API): display services, window services,
event services, and the vector type. All three services are real shell services with SDK clients; the pure models they
share live in `bifrost-ui` and are used by both sides of the wire.

Shared pieces

- `springboard/ui/src/wire_json.rs`: typed JSON field helpers and base64, used by the shell encoders and the SDK decoders
  so both sides agree on field names. `bifrost-ui` now depends on `bifrost-core` (no cycle: core has no dependencies).
- Permissions added to the manifest list (`springboard/src/bare/app_policy.rs`, `springboard/tools/aap.py`):
  `accessibility` (post events, event taps) and `display-config` (change the display mode). Services check the
  manifest on each call. Screen recording is not a manifest permission: it is the person's answer in Settings >
  Privacy (`privacy_permissions`), and the first capture call queues the Allow / Don't Allow sheet.

## Display services (service `display`)

- API: CGGetActiveDisplayList, CGGetOnlineDisplayList, CGMainDisplayID, CGDisplayBounds, CGDisplayPixelsWide/High,
  CGDisplayCopyAllDisplayModes, CGDisplayCopyDisplayMode, CGDisplaySetDisplayMode, mode refresh rate, CGDisplayRotation,
  CGDisplayIsInMirrorSet / CGDisplayMirrorsDisplay, CGDisplayIsBuiltin, CGDisplayVendorNumber / ModelNumber /
  SerialNumber, CGDisplayRegisterReconfigurationCallback, CGDisplayCreateImage.
- Paths: model `springboard/ui/src/display_info.rs`; shell `springboard/src/shell/services/display_services.rs`; SDK
  `springboard/sdk/src/displays.rs`; refresh rate fed from `springboard/src/bare/wayland.rs` (output mode event) into
  `OUTPUT_REFRESH_HZ`, preferring `Shell.info.display_mode` when the KMS path knows it.
- Done: active / online lists, main display, by id, point lookup, bounds in points, pixel size, mode list from the
  connector's sysfs `modes` and the current mode with refresh rate and scale, rotation from the shell orientation,
  mirror field (always none: one panel), built-in flag (connector name, battery as fallback), vendor / model / serial /
  physical size parsed from EDID (`parse_edid`), `reconfigured` event with per-display change flags (`diff`: moved,
  main, mode, added, removed, enabled, disabled, mirrored, unmirrored, desktop shape), screen capture as base64 PNG behind
  screen recording permission.
- Not done: `set_mode` records the requested mode and answers `pending` (the compositor owns the mode in a desktop
  session); rotation and mirroring set calls fail with a clear message (one panel). Per-mode refresh rates other than the
  current one are unknown (0).
- Check on Midget: the EDID read and connector choice on the real panel, refresh rate on the Wayland path, a capture
  while a window is open (the capture draws the shell frame, like a screenshot).

## Window services (service `windows`)

- API: CGWindowListCopyWindowInfo with the list options (on screen only, above / below window, exclude desktop
  elements), the window info dictionary keys (number, owner pid and name, bounds, layer, alpha, name, on screen,
  sharing state), CGWindowLevelForKey, CGWindowListCreateImage.
- Paths: model `springboard/ui/src/window_list.rs` (replaces the old pure list; same options, new record); shell
  `springboard/src/shell/services/window_services.rs` reading `Shell.desk.wins`; SDK `springboard/sdk/src/windows.rs`.
- Done: records from real shell state, front to back: menu bar, Dock, app windows (minimized ones off screen), native
  windows of other programs, the desktop; on the home screen the app in front. Titles are redacted for apps without
  screen recording permission except their own windows. 20 level keys with ordered layer numbers. Window image by number
  with the framing option and the opaque option, from the app's last frame when the compositor path keeps one, else a
  crop of the shell frame.
- Not done: bounds of native windows of other programs (the compositor does not report them to the shell, so they read
  as empty); window images of occluded windows without a kept frame show what is on top; per-window alpha is always 1
  (the shell does not expose effect alpha).
- Check on Midget: numbers and stacking order with several windows, minimized window, image of a full-size window, the
  crop with and without framing.

## Event services (service `events`)

- API: CGEventCreate, CGEventCreateMouseEvent, CGEventCreateKeyboardEvent, CGEventCreateScrollWheelEvent, CGEventGet/
  SetIntegerValueField / DoubleValueField, CGEventGet/SetFlags, CGEventGet/SetLocation, CGEventGet/SetTimestamp,
  CGEventPost, CGEventTapCreate (session level, listen-only and default), CGEventTapEnable / IsEnabled, CGEventSource
  (state ids, user data), CGEventSourceFlagsState / KeyState / ButtonState / SecondsSinceLastEventType / CounterForEventType.
- Paths: model `springboard/ui/src/event_tap.rs`; shell `springboard/src/shell/services/event_services.rs`; hooks in
  `Shell::event` (`springboard/src/shell/mod.rs`, pointer path) and `springboard/src/bin/shell.rs` (raw key and hover,
  posted input drained into the loop); SDK `springboard/sdk/src/input_events.rs`.
- Done: 15 event kinds, flags, key and button fields, scroll deltas, source identity, timestamps; field accessors; state
  tables for the hardware and session sources; tap list with head / tail placement, enable / disable, remove, listen-only
  and filtering taps; every input event runs through the taps (pointer, hover, raw keys); posting for mouse, hover,
  right-click, keys (modifiers pressed around them), modifier changes and scroll (as a short drag); taps removed with their
  app; the app is told what each tap saw (`tap` event). All behind the `accessibility` permission; nothing is tappable or
  postable while locked, and power / volume / wake keys never reach a tap.
- Not done: a filtering tap decides by declarative rules (drop, pass, replace key), because the shell cannot wait for an
  app in the middle of an input event; there is no callback-style filter. The wheel arrives in the shell already
  converted to a drag, so scroll wheel events are not seen by taps; text-only key events are mapped to a key by layout lookup.
- Check on Midget: a listen tap on the real keyboard, a drop rule on a key, a posted click on the dock, a posted
  command-key chord, rapid mouse movement load (notice cap is 64 per tick).

## CGVector

- `springboard/ui/src/geometry.rs`: `Vector` (dx, dy; length, dot, scaled, normalized, apply, between, add / sub / neg) and
  `Rect::offset_by`.

## Migrations and deletions

- `ui/src/window_list.rs` and `ui/src/event_tap.rs` were pure models with no callers outside their own tests. They are
  replaced in place (record and event types changed, no compatibility shim); no app or shell code used the old items.
- No app was changed, so no app version bump or changelog line.
