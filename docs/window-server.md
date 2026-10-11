# BifrOSt window server (`bifrost-wm`)

BifrOSt replaces labwc with its own Wayland compositor, written in Rust on [Smithay](https://github.com/Smithay/smithay).
The compositor is the window server: it owns window positions, stacking, focus and input routing, and it composites
every frame. Translucent materials blur the live content behind them at composite time, the way macOS WindowServer,
KWin and Mutter do it.

## Why

With labwc as the compositor, the shell is only a client. Wayland clients never learn where the compositor put or
moved their windows, so the shell's GPU presenter blurred a backdrop built from positions it guessed. A titlebar drag
(`xdg_toplevel.move`) moved the window inside labwc while the shell kept blurring the old spot: the glass stood still
while the window moved. Third-party apps could never get live glass at all. The fix is architectural: the process that
knows the geometry must draw the blur.

## Process model

`bifrost-wm` runs as the local account, exactly where labwc runs today (seatd for DRM and input, started by
`/usr/lib/bifrost/session`). The shell stays a separate, privileged client. Putting the compositor inside the shell
would make a root process the server for every untrusted app, so it stays out.

- Apps and the shell declare blur regions with the standard `ext-background-effect-v1` protocol and draw their own tint
  with alpha on top. The compositor blurs what is behind the region every frame it is damaged.
- Rounded material shapes and the BifrOSt glass styles (tint pairs, clarity mix) come in a later private extension
  (`bifrost_material_v1`); the standard protocol carries rectangles only.

## What labwc does today (parity checklist)

| Area | Today (labwc) | `bifrost-wm` |
|---|---|---|
| Core | `wl_compositor`, `wl_shm`, `wl_seat`, `wl_output`, `xdg_wm_base`, `zwp_linux_dmabuf_v1` | phase 1 |
| Shell surfaces | `zwlr_layer_shell_v1` (desktop background, menu bar, dock, menus) | phase 1 |
| Window management | placement, focus, raise, interactive move and resize, maximize, minimize, close | phase 1 |
| Decorations | server-side for third-party apps (BifrOSt theme, close/iconify/max), none for `is.olibuijr.bifrost.*` | phase 3 |
| Glass | none (the shell faked it) | phase 2: `ext-background-effect-v1`, live dual-Kawase blur |
| Lock | `ext_session_lock_manager_v1` | phase 3 |
| Idle and power | `ext_idle_notifier_v1`, `zwlr_output_power_manager_v1` | phase 3 |
| Cursor | `wp_cursor_shape_manager_v1`, Adwaita 32 | phase 3 |
| Activation | `xdg_activation_v1` | phase 3 |
| Window list | `zwlr_foreign_toplevel_manager_v1` (dock, app switcher) | phase 3 |
| Output modes | `wlr-output-management` via `wlr-randr` (`video-arg`, `session-env`) | phase 3; the compositor picks the fastest native mode itself |
| Screenshots | `wlr-screencopy` via `grim` (`bifrost-apps --screenshot`) | phase 3 |
| Keys | Super+Space (Spotlight via control port), Super+A (apps), Alt+Tab, Alt+F4 | phase 3 |
| X11 apps | XWayland | phase 4 |
| Backend | DRM/KMS, libinput, libseat | phase 3 (phase 1 runs nested for development) |

## Phases

1. **Core, nested.** Smithay with a nested backend for development on any Wayland desktop and a headless backend for
   tests. Core protocols, layer shell, xdg-shell window management with interactive move and resize, GLES rendering
   with damage tracking.
2. **Live glass.** `ext-background-effect-v1`: per-surface blur regions, blurred from the composited content behind the
   surface every damaged frame, dual-Kawase like the shell's existing material.
3. **Session.** DRM/KMS + libinput + libseat backend, lock, idle, output power, cursor shapes, activation, foreign
   toplevels, output management, screencopy, keybindings, server-side decorations.
4. **XWayland** and the acceptance list in `docs/wayland-desktop.md`.
5. **Switch.** The shell drops its own backdrop blur and requests regions instead; the session script starts
   `bifrost-wm`, with labwc as the fallback until acceptance passes on Midget.

## Code

`springboard/wm` (crate `bifrost-wm`). Window-management rules (placement, stacking, focus, move and resize
grabs) live in plain Rust modules without Smithay types, so they are unit-tested on any host.

## Presenter bugs the window server retires

The shell-side GPU presenter (`springboard/src/gpu/wl.rs`) fakes glass inside its own buffers. Bugs of that design,
found on Midget on 2026-10-11:

- **Frozen glass while dragging.** labwc moves the window (`xdg_toplevel.move`); the presenter keeps blurring the old
  spot. Only the compositor knows the geometry.
- **Burn-in after the full-screen app switcher.** `bifrost_render::glass::merge` drops a stored shape only when it lies
  wholly inside one damage rectangle, and keeps partly damaged shapes on purpose (the clock redraws a strip of the bar
  without re-reporting the bar). A shape that disappears while only partly damaged, or damaged as several rectangles,
  stays in the list, and the presenter keeps drawing its blur and tint on the desktop.
- **Black desktop after unlock** (fixed in `cbd807d`): the lock unmapped the desktop layer surface and the unlock
  attached a buffer before the new configure.

## Open desktop issues (user notes, 2026-10-11)

1. Dock icons vibrate under hover magnification; it must be smooth, and every dock setting belongs in Settings.
2. Files: the empty-folder screen needs a proper Golden Gate design.
3. Super+Space does not open Spotlight (labwc keybinding today; `bifrost-wm` phase 3 owns keys).
4. Switching windows: the dimming of inactive windows should fade in and out, in both directions.
5. The dock icon context menu is oversized; follow the HIG menu metrics.
6. The burn-in above.
