# Wiring: `cursor` system service (cursor stack)

Matrix rows: `appkit/nscursor` (AppKit, User Interfaces and User Interactions, "Menus, Cursors, and the Dock"; slug
`appkit-user-interactions` / `appkit-user-interface`). Model: `springboard/ui/src/cursor.rs` (`Kind`, `Cursors`).

## What is wired now

- `springboard/src/shell/services/cursor_service.rs`: the `cursor` service. A process-wide `Cursors` stack, with
  methods `push(shape)`, `pop()`, `set(shape)`, `current()`, `depth()`, `hide()`, `unhide()`,
  `set_hidden_until_mouse_moves(bool)`. Shapes are `Kind::name()` strings (`i_beam`, `resize_left_right`, ...).
- `springboard/sdk/src/cursor_service.rs`: typed client (`push`, `pop`, `set`, `current`, `depth`, `hide`, `unhide`,
  `hide_until_moved`, `show_now`) over `service::call`. No wire-protocol change.
- `springboard/src/bare/wayland.rs`: the desktop-wayland pointer applies the stack. A pushed shape replaces the
  shell's own window-edge resize shape (`wl_shape` maps each `Kind` to the cursor-shape protocol). Hidden pointers
  get `wl_pointer.set_cursor(serial, None)`. Every motion calls `pointer_moved()`, so hide-until-moved ends on a move.
- `Kind::name` / `Kind::from_name` in `springboard/ui/src/cursor.rs` give the one spelling both sides use.

## How a user reaches it

Apps call the SDK client from their handlers, for example a text view pushes `IBeam` on enter and pops on leave.
Users do not configure it directly.

## How to verify on the test PC (Midget)

1. `bifrost pc midget state` shows the shell; the service list includes `cursor` (`system.services`).
2. From an app, `cursor_service::push(ctx, Kind::PointingHand)`, then move the pointer over a control.
   On the Midget the pointer is the shell's software sprite (see gaps), so the shape change is not visible there.
   Check the service state instead with `bifrost pc midget state`, and `cursor_service::pop` restores it.
3. Host test: `cargo test -p bifrost-ui cursor` and the `shell::services::cursor_service` tests (not run in this round).

## REA evidence

- `python3 -I tools/parity/docc.py appkit/nscursor`: push, pop, set, hide, unhide, setHiddenUntilMouseMoves, the
  stack and the system cursor accessors. No REA page evidence was taken in this round (`rea-page.sh` not run).

## Remaining gaps

- The Midget (bare KMS) pointer is the shell's own software sprite (`gfx::cursor`). It ignores the stack, so
  pushed shapes and hides do not show on bare hardware yet. Only the Wayland desktop path applies them.
- No cursor images or hot spots (NSCursor's `image` and `hotSpot`): the model stores shape names only.
- A stack entry is not removed when the app that pushed it exits, so a crashed app can leave a shape pushed.
- Build and test not run in this round (wave rule: the lead builds once).
