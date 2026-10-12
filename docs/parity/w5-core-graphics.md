# Parity wave 5: Core Graphics (window and event services)

Two gap rows of the Core Graphics framework now have a pure Rust model in `springboard/ui/src`. Both are pure
models with no I/O: they are not wired into the window server, the shell or any app, and they have not been built or
run on the VM or the Midget (no builds were run for this wave).

## What was built

- `springboard/ui/src/window_list.rs` (`bifrost_ui::window_list`): the window records an app can read (id, owner,
  title, layer, bounds, on-screen flag, alpha), listed front to back. `WindowScope` (all, on-screen only, on-screen
  above a window, on-screen below a window) and `exclude_desktop_elements` select the list. A desktop element is a
  window on a layer below `NORMAL_LAYER` (0). Helpers: `stacking_index`, `is_above`. The window server still supplies
  the records; no SDK call reads them yet.
- `springboard/ui/src/event_tap.rs` (`bifrost_ui::event_tap`): event kinds (`EventType`), event masks (`EventMask`),
  synthetic events (`Event::mouse`, `Event::key_down` built on `keys::KeyCombo`), and an ordered tap list
  (`EventTapList`). A tap is created at the head or the tail of the list, runs in `DefaultTap` or `ListenOnly` mode,
  and answers each event with `Decision::Pass`, `Drop` or `Replace`. `post` runs an event through the enabled taps
  whose masks watch its kind and returns the event that reaches the stream, or `None` if a tap dropped it. Taps can be
  enabled, disabled and removed by id.

Each module has a `#[cfg(test)] mod tests` block with behaviour tests worked out by hand from the rules above.

## Apple API names mirrored

- Window services: `CGWindowListCopyWindowInfo` (option set `CGWindowListOption`: `optionAll`,
  `optionOnScreenOnly`, `optionOnScreenAboveWindow`, `optionOnScreenBelowWindow`, `excludeDesktopElements`).
  Official page: https://developer.apple.com/documentation/coregraphics/quartz-window-services
- Event services: `CGEventTapCreate` (placement `CGEventTapPlacement.headInsertEventTap` and `tailAppendEventTap`,
  options `CGEventTapOptions.defaultTap` and `listenOnly`), `CGEventMask`, `CGEventType`, `CGEvent` init for mouse and
  keyboard events, `CGEventTapCallBack`, `CGGetEventTapList`, `tapEnable`/`tapIsEnabled`.
  Official page: https://developer.apple.com/documentation/coregraphics/quartz-event-services
  Option and type pages consulted: https://developer.apple.com/documentation/coregraphics/cgwindowlistoption,
  https://developer.apple.com/documentation/coregraphics/cgeventtapoptions,
  https://developer.apple.com/documentation/coregraphics/cgeventtapplacement,
  https://developer.apple.com/documentation/coregraphics/cgeventtype

Differences from the official API, all deliberate for this wave:
- Event masks use this crate's own bit numbering, one bit per `EventType` in declaration order, not the window
  server's numbering.
- Taps are not created in a live event stream and are not per-process (`tapCreateForPSN` has no counterpart).
- The desktop-element rule (layer below the normal layer) is this desktop's rule, not a copy of the official constant
  values. The official window-level key table is not mirrored.
- `optionIncludingWindow`, the event source state, field get/set and the Unicode string calls are not modelled.

## REA evidence

- Quartz Window Services page: `ev_d7f28fc307d42d2ca3a4111fa0614b272770177e4fc931d057f479a2352a27b6`
- Quartz Event Services page: `ev_aa1a86f6293584a5957b6c0145a77e68cba1e61420d07dca34bb104d20e832a7`

## Matrix rows closed

These rows move from `gap` to `partial`. The lead updates `docs/parity-matrix/` centrally.

| Framework slug | Feature (as in the matrix data) | New status | BifrOSt path |
|---|---|---|---|
| core-graphics | Window services (Quartz Window Services, CGWindowList) | partial | springboard/ui/src/window_list.rs |
| core-graphics | Event services (Quartz Event Services, CGEventTap, synthetic events) | partial | springboard/ui/src/event_tap.rs |

## Still open

- No SDK call exposes the window list or the event taps to apps; the window server does not feed the models.
- The models are not built or tested on the VM or Midget yet.
