# Graphics rebuild: waves

The architecture is fixed in `docs/graphics-architecture.md`; these waves only order the work by dependency. Every
wave: Haiku 5.5 agents in worktrees (20 per wave), `cargo check -p <crate>` allowed, the lead writes the skeleton and
contracts first, reviews and fixes every branch before merging, then builds, tests and tries it on Midget.

## G1 (running): the building blocks

`bifrost-gpu` (device, textures and dma-bufs, memory and sync, commands, pipelines), `Canvas` and `DisplayList`,
`bifrost-2d` (shapes, text, images and paths, renderer), `bifrost-layers` (model, animation, codec, protocol), the
`bifrost-wm` render server modules (clock and stats, import and output, compose, material, layers) and the SDK's
direct path (connection and backing, input and frame). Nothing is wired into a running process yet.

## G2: make it run end to end

Goal: on Midget, `bifrost-wm` composites the desktop with Vulkan, at least one SDK app draws on the GPU and talks to
the window server directly, and the bench shows the frame budgets. Facts this plan rests on (0.5.44 + G1):

- `wm/src/drm.rs` renders through Smithay's `GlesRenderer` and `DrmCompositor` (`Drm::render` builds `scene::build`
  elements, `render_frame`, `queue_frame`); `nested.rs` uses winit with GLES; `glass.rs` is the GLES blur.
- `wm/src/handlers.rs` implements compositor, shm, seat, data device, output, xdg-shell, layer-shell, idle-notify and
  dmabuf only. Not yet: presentation-time, foreign-toplevel, screencopy, cursor-shape, xdg-activation, output
  management, decorations, pointer gestures and constraints, explicit sync, XWayland, keybindings.
- `App::draw` takes `&mut Surface` in 58 places across 21 apps; the SDK kit draws on `Surface` in 12 files
  (`widgets.rs` 50 uses, `glyphs.rs`, `view/*`).
- The shell hosts 9 built-in apps in-process (`AppId`: Settings, Terminal, Deildu, Photos, Files, Camera, Browser,
  Magnifier, AppStore) and draws the menu bar, Dock and panels itself (`desktop_bar.rs`, `desktop_dock.rs`,
  `control_center.rs`, `notify/`), all on one event loop.

Lead skeleton before launch: the `render_server::Server` entry point (`new(device, outputs)`, `frame(state, now)`),
`App::draw_canvas` with its default bridge, the menu-model control messages for the chrome clients, the new crates
`springboard/chrome` (menu bar, Dock and panels clients), and the `tests/vulkan_*.rs` harness layout.

| # | Agent | Owns | Work |
|---|---|---|---|
| 1 | `wm-drm-vulkan` | `wm/src/drm.rs` | Vulkan render server on the DRM backend: device on the scanout GPU's render node, GBM output targets, atomic commit with the submission's in-fence, the frame loop driven by `FrameClock`; GLES stays as the fallback when Vulkan fails. |
| 2 | `wm-nested-headless` | `wm/src/nested.rs`, `headless.rs`, `tests/vulkan_*.rs` | The Vulkan server on the nested and headless backends; headless renders to a texture and reads it back for end-to-end tests (window, layer tree, blur, damage) behind `gpu-tests`. |
| 3 | `wm-scene` | `wm/src/scene.rs`, `render_server/compose.rs` glue | Port scene building (surface trees, subsurfaces, popups, layer-shell, cursor, boot fade) to `DrawItem`s; window shadows and glass through `material`. |
| 4 | `wm-sync-feedback` | new `wm/src/sync.rs`, handlers glue | linux-dmabuf v4 feedback from the device's formats, `wp_linux_drm_syncobj_v1` explicit sync (acquire and release points on `SharedEvent`), `wp_presentation` feedback from the clock. |
| 5 | `wm-session` | new `wm/src/session_protocols.rs` | foreign-toplevel (Dock, switcher, MCP), screencopy (ext-image-copy-capture plus wlr-screencopy for grim and `bifrost-apps --screenshot`), cursor-shape, xdg-activation, wlr-output-management. |
| 6 | `wm-windows` | `wm/src/wm.rs`, `grabs.rs`, new `decorations.rs` | labwc's keybindings (Ctrl+Super arrows tiling, Super+Space, Alt+Tab, Alt+F4), xdg-decoration with server-side titlebars for third-party apps, window open, close and minimize animations run by the server (minimize targets the Dock icon rect the Dock client publishes). |
| 7 | `wm-input` | `wm/src/input.rs` | libinput configuration from the shell's prefs (acceleration, natural scrolling, tap), pointer-gestures (swipe, pinch, hold), pointer constraints and relative pointer, key repeat, cursor theme and size. |
| 8 | `wm-xwayland` | new `wm/src/xwayland.rs` | XWayland via Smithay's xwayland support, X11 windows in the same stack. |
| 9 | `kit-canvas` | `sdk/src/widgets.rs`, `glyphs.rs` | The SDK kit draws through `&mut dyn Canvas` (Surface still works because it implements Canvas). |
| 10 | `kit-canvas-views` | `sdk/src/view/*` | Views, controls, forms, scroll and animation draw through `Canvas`. |
| 11 | `sdk-draw-canvas` | `sdk/src/lib.rs` App trait, `restoration.rs`, `webview.rs`, testapp | `App::draw_canvas(ctx, &mut dyn Canvas, t)`, defaulting to `draw` through a `Surface` bridge; migration guide in `docs/bifrost-ui.md`. |
| 12 | `sdk-layer-views` | new `sdk/src/layers.rs`, `view/scroll.rs` integration | Layer-backed views: views map to layers; scrolling is a layer offset with server-run deceleration; `animate(duration, curve, \|\| ...)` sets the transaction's implicit animation; hover and press are property changes. |
| 13 | `sdk-direct-runtime` | `sdk/src/runtime.rs` | The runtime runs the direct path when `BIFROST_DIRECT=1`: one loop over the Wayland fd and the shell control socket; pixels and input never go to the shell. |
| 14 | `shell-apphost-direct` | `src/shell/apphost.rs`, `desk.rs` glue | The shell stops making a toplevel for a direct app; window list from foreign-toplevel, switcher and crash thumbnails from screencopy. |
| 15 | `chrome-menubar` | `springboard/chrome/src/menubar.rs` | The menu bar and its menus as a separate layer-shell client drawn with `Canvas` + `bifrost-2d` + layers; the menu model comes from the shell over the control socket. |
| 16 | `chrome-dock` | `springboard/chrome/src/dock.rs` | The Dock as a separate client: layer-tree magnification, launch bounce and minimize target published to the window server. |
| 17 | `chrome-panels` | `springboard/chrome/src/panels.rs` | Notification Center, Control Center and search as one panels client, with glass materials from the window server. |
| 18 | `app-store-out` | new `apps/appstore-desktop` | The App Store as an SDK app (from `src/shell/store/`), first of the built-in apps to leave the shell. |
| 19 | `bench-graphics` | `tools/bench.py` graphics suite | Midget: window-server frame stats (`bifrost-wm-frames.jsonl`), input-to-photon (uinput click to the first presented frame that shows it), animation drops, app redraw time; budget gate against `docs/graphics-architecture.md`. |
| 20 | `session-switch` | `packaging/arch/wayland/session`, PKGBUILD, Settings > Developer | Ship `bifrost-wm` and its shaders, a Developer toggle for the new window server and direct apps, and a crash-loop fallback to labwc. |

Exit criteria for G2: the Vulkan server boots on Midget with the toggle, the desktop and one direct app draw, glass
and shadows show, `bench --midget` reports the frame budgets, and the GLES and labwc paths still work.

## G3: move everything over

All 21 apps draw through `draw_canvas` and run direct; Settings, Photos, Camera, Terminal, Files, Browser, Magnifier
and Deildu leave the shell as SDK apps; the shell keeps only the session, services, permissions and the control
socket; the menu bar, Dock and panels run as chrome clients by default; foreign-toplevel replaces the shell's own
window list; snapshots are blessed from GPU renders.

## G4: switch and remove

Midget acceptance (`docs/wayland-desktop.md`) and the budgets pass for a week of use; the session starts
`bifrost-wm` only; labwc, the shell's per-app toplevels, the shell-side presenter (`src/gpu/wl.rs`) and the
shared-memory frame path for direct-capable apps are removed (the CPU path stays only for machines without Vulkan);
docs, the website architecture chapter and the parity rows are updated.
