# G2 wm-xwayland: X11 apps in the window server's stack

Owner: wave G2 agent `wm-xwayland`. Contract: `docs/graphics-architecture.md` (window server, XWayland).
Row: `docs/graphics-waves.md`, G2, item 8.

## What is implemented

`springboard/wm/src/xwayland.rs` (feature `xwayland`, which is on by default in `bifrost-wm`):

- `start(handle, state, on_ready)`: spawns XWayland with Smithay's `XWayland::spawn` (abstract socket on, stdio
  discarded, display number chosen by Smithay). On `XWaylandEvent::Ready` it stores the display (`:N`), starts the X11
  window manager with `X11Wm::start_wm` on XWayland's X connection and runs `on_ready`. On `Error`, or when the spawn
  fails, `on_ready` runs at once, so the client still starts without X11. `BIFROST_XWAYLAND=0` skips XWayland.
- `display()`: the `DISPLAY` value once XWayland is ready. `lib.rs::spawn` sets `DISPLAY` from it for every client the
  compositor starts (before, it always removed `DISPLAY`).
- `XWaylandState` on `WmState` (`state.rs`, field `xwayland`): the xwayland-shell global (`XWaylandShellState`), the
  XWayland client and the `X11Wm`.
- `impl XwmHandler for WmState`:
  - `map_window_request`: a managed X11 window is placed by `wm` like a new toplevel (`wm.map`), configured to that
    frame, mapped in the `Space` and focused.
  - `mapped_override_redirect_window` / `configure_notify` for override-redirect windows: mapped at the location they
    ask for, above the managed windows (menus, tooltips, combo boxes). They are not reconfigured by `wm`.
  - `configure_request`: managed windows are granted the `wm` frame; override-redirect windows are left to XWayland.
  - `configure_notify` for managed windows: a client-side size change becomes the frame (its position is kept).
  - `unmapped_window` / `destroyed_window`: `wm.unmap`, unmap from the `Space`, focus the next window.
  - `move_request` / `resize_request`: a pointer grab (`MoveGrab` / `ResizeGrab`, the same grabs as Wayland toplevels)
    started from the pointer's current location, with the edges from the X11 request.
  - `disconnected`: drops the `X11Wm`.
- `impl XWaylandShellHandler` and `delegate_xwayland_shell!`.
- `client_compositor_state`: the XWayland client's compositor state (its client data is `XWaylandClientData`, not
  `state::ClientState`).

Changes outside `xwayland.rs` (small, local hooks):

- `state.rs`: the `xwayland` field (`cfg(feature = "xwayland")`); `window_for_surface` and the keyboard focus now go
  through `WaylandFocus::wl_surface()`, so X11 surfaces commit, focus and resolve like Wayland ones. Before, they
  depended on `toplevel()`, which an X11 window never has.
- `handlers.rs`: `client_compositor_state` falls back to the XWayland client data (without it, the first X11 surface
  commit panics).
- `lib.rs`: `pub mod xwayland` and the `DISPLAY` rule in `spawn`.
- `main.rs` (nested backend): the command is spawned from `xwayland::start`'s `on_ready`, so it sees `DISPLAY`.

Unit tests (`xwayland.rs`, `mod tests`): geometry of the frame, the edge mapping for each of the eight resize edges,
corner edges moving two sides, no opposite sides moving together, `display()` unset before XWayland is ready.

## Mapping to the contract

- The window server stays one `wm` + one `Space`: X11 windows are `Window::new_x11_window` elements, so they are drawn by
  the same render path, hit-tested by `surface_under` and stacked by `wm`.
- Input goes straight to the focused client surface through the seat, as for Wayland. The X11 window manager only
  sets X input focus and activation (`Window::set_activated` already handles X11).
- The old path is unchanged when XWayland is not started: `BIFROST_XWAYLAND=0`, a missing `Xwayland` binary, or the
  DRM backend (see todo_left) all run the command at once.

## What is left (todo_left)

- Clipboard bridging: `XwmHandler::new_selection`, `send_selection` and `cleared_selection` are the Smithay defaults
  (no-ops). Smithay's `X11Wm::new_selection` and `send_selection` plus `set_data_device_selection` /
  `request_data_device_client_selection` are the pieces to wire up (needs `SelectionHandler::new_selection` in
  `handlers.rs`).
- DRM backend: `drm.rs` still calls `crate::spawn` directly with no XWayland start, so X11 apps do not run there yet.
  The hook is one `xwayland::start` call before its `spawn` plus an event-loop run.
- Raising: `focus()` raises the window in the `Space` but does not call `X11Wm::raise_window`, so X server stacking can
  lag behind the compositor's.
- Maximize / fullscreen / minimize requests from X11 clients are ignored (the window keeps its `wm` frame).
- The xwayland-shell global is registered for every client, not filtered to the XWayland client
  (`XWaylandShellState::new`); a filter needs `create_global_with_filter`, which Smithay 0.7 does not expose here.
- Override-redirect windows are mapped in the `Space` above the managed windows, not as xdg popups; their pointer and
  keyboard handling is the generic `Space` path.
- No headless or nested test runs yet: the unit tests cover the pure logic only.

## How to verify on Midget

Midget: Intel HD 530, Mesa ANV, Vulkan 1.4. The session runs labwc unless `/etc/bifrost/compositor` says `bifrost-wm`.

1. Build the fast profile: `cargo build --offline --profile fast --bin bifrost-wm` in `springboard/` (the nested backend
   is on by default). Push it with `tools/midget-push-shell.sh --no-build shot.png` or the usual push.
2. On Midget, with `bifrost-wm` as the session compositor, check the log for `bifrost-wm: XWayland on DISPLAY=:N`.
   If it is missing, `Xwayland` is not on PATH (`which Xwayland`) or the spawn failed.
3. Start an X11 client from the compositor's command line, for example `xeyes` or `xterm`, and check that:
   - `echo $DISPLAY` inside it is `:N`;
   - its window appears in the stack, can be focused by clicking, moved from its title area and resized from an edge;
   - a menu of an X11 app (for example from `xterm`'s context menu on a toolkit app) maps above the window.
4. Compare with `BIFROST_XWAYLAND=0`: the client starts with no `DISPLAY` and the Wayland path is unchanged.
5. Take a Midget screenshot with `bifrost pc midget state` and `screen PATH` for the record.

Host check: `CARGO_TARGET_DIR=... cargo check --offline -j 2 -p bifrost-wm --tests` in `springboard/`.
