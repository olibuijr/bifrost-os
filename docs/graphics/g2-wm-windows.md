# G2 `wm-windows`: window rules, decorations and server-run window animations

Owner: `wm-windows` (wave G2 of the graphics core, `docs/graphics-waves.md`). Branch `g2/wm-windows`.

## What is implemented

Only the pure, Smithay-free parts. All of it is in `springboard/wm/src` and builds with `cargo check`. The
tests were written with hand-worked expected values but were not run by this agent (the build rules forbid
`cargo test`), so run them first (see Verify).

- `wm.rs`: tiling rules for the labwc keys.
  - `Side { Left, Right }` and `half(area, side)`: the left and right halves of the usable area; the right
    half takes the odd pixel.
  - `Wm::snap(id, side)` (Ctrl+Super+Left and Right): keeps the frame from before the first snap, moves the
    window to a half. `snapped(id)` reports the side.
  - `Wm::restore_centre(id)` (Ctrl+Super+Down): leaves maximized or snapped state and centres the window at its
    previous size (or its current size when never moved aside).
  - `toggle_maximize` (Ctrl+Super+Up): from a snap it fills the area and keeps the pre-snap frame; the next toggle
    returns to that frame.
  - `is_maximized` is false for a snapped window. `set_area` refits snapped windows to their half.
  - `set_frame`, `begin_move` and `begin_resize` clear the snap state.
- `decorations.rs` (new): server-side titlebar policy and geometry.
  - `mode_for(app_id)`: `Mode::None` for `is.olibuijr.bifrost.*` app ids, `Mode::ServerSide` otherwise.
  - `titlebar_of(frame)` (28 px strip above the frame), `lights(bar)` (close, minimize and zoom squares, 12 px,
    8 px gap, 12 px from the left), `title_rect(bar)` (centred between the lights and an equal right margin),
    and `hit(bar, x, y)` returning `Hit::{Close, Minimize, Zoom, Drag, Outside}`.
- `render_server/window_anim.rs` (new): server-run open, close and minimize animations.
  - `Kind::{Open, Close, Minimize}`; open and close use 250 ms with ease-out cubic and scale 0.9 to 1 (reverse
    for close) with a fade; minimize uses 350 ms from the window frame to the Dock rect.
  - Reduce motion: open and close become a 120 ms fade with no scale; minimize is instant. The caller reads the
    flag from `/etc/bifrost` or the shell prefs.
  - `Pose { sx, sy, dx, dy, alpha }` with `apply(frame) -> Rect`.
  - `Animator`: one animation per window, `pose`, `running` (a closing window keeps its last buffer while
    `running` is `Close`), `sweep` (returns finished closes and minimizes), `active`, `next_deadline`.
- Module hooks: `pub mod decorations;` in `springboard/wm/src/lib.rs`, and `pub mod window_anim;` in
  `springboard/wm/src/render_server/mod.rs`.

## Contract mapping

| Contract item | Status |
|---|---|
| labwc keybindings (Ctrl+Super arrows, maximize, centre) | rules done in `wm.rs`; key binding and dispatch not wired |
| Super+Space, Super+A to the shell control port | not done (input layer, `input.rs`) |
| Alt+Tab cycle, Alt+F4 close | not done (input layer) |
| xdg-decoration, server-side titlebars as compose nodes | policy and geometry done in `decorations.rs`; no xdg-decoration handler, no compose nodes |
| Window open, close and minimize run by the server | timing and pose done in `window_anim.rs`; not driven from the render loop |
| Minimize to the Dock's published rect | `Anim` takes the target rect; the Dock publish path is not wired |
| Reduce motion from /etc/bifrost or shell prefs | flag passed in `Anim::new`; the reader is not written |

## todo_left

- Wire the snap and centre keys into `input.rs` and the key handler, and the Ctrl+Super+Up toggle.
- Add the xdg-decoration handler (`handlers.rs`) that asks for server-side mode per app id, and draw the titlebar
  as compose nodes with `lights`, `title_rect` and `hit` (click handling in the pointer path).
- Drive `Animator` from the render loop: start `Open` on map, delay the unmap of a window until `Close` is swept,
  and start `Minimize` with the Dock rect the Dock publishes.
- Read the reduce-motion flag from `/etc/bifrost` and the shell prefs.
- Alt+Tab cycle and Alt+F4 close; Super+Space and Super+A through the shell control port.
- Run the new unit tests with `cargo test -p bifrost-wm` on a host (not run here).

## Verify

Host (after the build rules allow tests):

    cd springboard && cargo test -p bifrost-wm wm:: decorations:: render_server::window_anim::

Expected: all tests pass. The snap and centre tests are in `wm.rs`; the titlebar hit tests in `decorations.rs`; the
timing and pose tests in `window_anim.rs`.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): the session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says
`bifrost-wm`. After the wiring above lands, select the new path with `BIFROST_WM_RENDERER=vulkan` and
`/etc/bifrost/compositor`, then check snap with Ctrl+Super+Left and Right, maximize with Ctrl+Super+Up, and
centre with Ctrl+Super+Down. Check the open and close animations and that the minimize lands on the Dock icon.
Until the wiring lands, none of this is visible on Midget.
