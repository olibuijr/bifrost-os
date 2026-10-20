# BifrOSt window server (`bifrost-wm`)

BifrOSt's own Wayland compositor (it replaced labwc, which is removed), written in Rust on [Smithay](https://github.com/Smithay/smithay).
The compositor is the window server: it owns window positions, stacking, focus and input routing, and it composites
every frame. Translucent materials blur the live content behind them at composite time, the way macOS WindowServer,
KWin and Mutter do it.

## Why

With a third-party compositor (labwc, removed), the shell is only a client. Wayland clients never learn where the compositor put or
moved their windows, so the shell's GPU presenter blurred a backdrop built from positions it guessed. A titlebar drag
(`xdg_toplevel.move`) moved the window inside that compositor while the shell kept blurring the old spot: the glass stood still
while the window moved. Third-party apps could never get live glass at all. The fix is architectural: the process that
knows the geometry must draw the blur.

## Process model

`bifrost-wm` runs as the local account, as the only window server (seatd for DRM and input, started by
`/usr/lib/bifrost/session`). The shell stays a separate, privileged client. Putting the compositor inside the shell
would make a root process the server for every untrusted app, so it stays out.

- Apps and the shell declare blur regions with the standard `ext-background-effect-v1` protocol and draw their own tint
  with alpha on top. The compositor blurs what is behind the region every frame it is damaged.
- Rounded material shapes and the BifrOSt glass styles come through the private `bifrost_material_v1` protocol (see
  Materials); the standard protocol carries rectangles only.
- Accepted Wayland sockets cache their Linux `SO_PEERCRED` uid in `ClientState`. The input-method and virtual-keyboard
  global filters use that cached uid: Wayland invokes the filters while holding its backend lock, so querying
  `Client::get_credentials` from a filter re-enters the lock and deadlocks client dispatch. Missing cached credentials
  (including foreign XWayland client data) fail closed under the same-user policy.

## What the former compositor did (parity checklist)

| Area | Former (labwc) | `bifrost-wm` |
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
| Screenshots | `wlr-screencopy` via `grim` (`bifrost-apps --screenshot`) | done: readback in `wm/src/capture.rs` (see Screen capture) |
| Keys | Super+Space (search panel via control port), Ctrl+Up and F3 (Mission Control via control port), F4 (Apps grid via control port), Alt+Tab, Alt+F4, Ctrl+Super+arrows | done: `wm/src/keybindings.rs` (see Key bindings) |
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
   `bifrost-wm`, the only window server; the fallback is `bifrost-wm` safe mode (GLES, no direct apps, no chrome clients) after three quick exits, then the recovery flow.

## Boot: one modeset, one fade

`bifrost-wm` keeps the boot policy of `docs/bifrost-boot-design.md`:

- It opens the GPU without resetting connectors and keeps the mode the CRTC already shows (the loader's, or the
  learned `video=` mode), so the loader's modeset stays the boot's only one, as the former compositor's `reuseOutputMode` did.
  Only without an active mode does it pick the native size at its fastest refresh. `BIFROST_WM_MODE=WxH@R` overrides.
- Rotation: `BIFROST_ROTATION`, else the first line of `/etc/bifrost/rotation`, names the output transform (`normal`,
  `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`, `flipped-270`). It is set on the output, so a 1200x1920
  panel with `90` shows a 1920x1200 desktop to clients, the layer map, the lock surface and captures. The Vulkan
  scanout does not rotate, so a rotated output stays on the GLES scanout. `transform.rs` holds the point and
  rectangle mapping between the desktop and the panel for absolute input (touch).
- With `BIFROST_BOOT_FADE` it shows the loader's frame (`bootsplash::paint`, the shell's own file) from its first
  scanout, holds it until the shell's first desktop frame (`$XDG_RUNTIME_DIR/shell-ready`), then cross-fades over
  450 ms with the loader's smoothstep, writing the `desktop-first-ui` and `desktop-faded` marks. The compositor owns
  the screen, so under `bifrost-wm` the shell no longer fades itself.
- `video-arg` keeps learning the fastest mode for the next boot's command line, and `display-refresh` (session autostart) picks the fastest refresh at native size. Both still read the modes with `wlr-randr`; moving them to `bifrost-wm` output management is open.

## Code

`springboard/wm` (crate `bifrost-wm`). Window-management rules (placement, stacking, focus, move and resize
grabs) live in plain Rust modules without Smithay types, so they are unit-tested on any host.

## Presenter bugs the window server retired

History. The shell-side GPU presenter (`springboard/src/gpu/wl.rs`) faked glass inside its own buffers. The live
backdrop (`Presenter::under`, `BIFROST_WM_MATERIALS`) was deleted with the labwc removal; the window server is the only
place glass is composed now. Bugs of that design, found on Midget on 2026-10-11:

- **Frozen glass while dragging.** the old compositor moved the window (`xdg_toplevel.move`); the presenter keeps blurring the old
  spot. Only the compositor knows the geometry.
- **Burn-in after the full-screen app switcher.** `bifrost_render::glass::merge` drops a stored shape only when it lies
  wholly inside one damage rectangle, and keeps partly damaged shapes on purpose (the clock redraws a strip of the bar
  without re-reporting the bar). A shape that disappears while only partly damaged, or damaged as several rectangles,
  stays in the list, and the presenter keeps drawing its blur and tint on the desktop.
- **Black desktop after unlock** (`cbd807d`, again 0.10.4): the lock unmapped the desktop layer surface with a null-buffer
  commit and the unlock waited for a new configure. bifrost-wm (smithay) resets a layer surface's configure state only
  when it is destroyed, so it never answers the fresh initial commit and the desktop stayed unmapped (the window server's
  clear colour, `0.06 0.06 0.08`, with no wallpaper and no backdrop for the window materials) on every second login. The
  shell no longer unmaps the desktop surface at a lock: bifrost-wm draws only the lock surface while locked anyway.

## Desktop issues (user notes, 2026-10-11)

1. Dock icons vibrate under hover magnification; it must be smooth, and every dock setting belongs in Settings.
2. Files: the empty-folder screen needs a proper Golden Gate design.
3. Super+Space did not open the search panel (a former labwc keybinding). Fixed: `bifrost-wm` owns the chord and sends `quicksearch` to the shell (see Key bindings).
4. Switching windows: the dimming of inactive windows should fade in and out, in both directions.
5. The dock icon context menu is oversized; follow the HIG menu metrics.
6. The burn-in above.


## Live blur per compositor path (2026-10-12)

Every built-in shell window (Settings and the rest) is already its own xdg toplevel (`src/bare/wayland.rs`, app id
`is.olibuijr.bifrost.N`), not a region of one shell surface. What differs is who composes the blur.

| Path | Who blurs | What the blur sees |
|---|---|---|
| labwc (removed, history) | the shell's presenter painted the blur into the window's own buffer (`Backdrop::GlassOnly`, `Presenter::under`; both deleted) | wallpaper plus the shell's own windows only: Settings over Files showed the wallpaper, not Files. |
| bifrost-wm | the window server (`wm/src/scene.rs`, `glass.rs`): each surface with an `ext-background-effect-v1` blur region gets a backdrop rendered from every element below it (all clients), re-rendered when the signature of those elements changes (window moved, client committed) | the real composed scene, including Files |

Shell side (this round): the shell binds `ext_background_effect_manager_v1` when the compositor offers it and reports
the blur capability. For each window with materials it then sends the material rects (shadow margin added) as the
blur region and leaves the window translucent: no presenter blur, no inactive opaque plate.
`BIFROST_WM_MATERIALS` and the presenter path no longer exist. The SDK apps (`bifrost_app`) must use the same call; no client
sends a region yet except the shell and, through it, SDK apps (step 3 below).

### Former labwc path: black Files and black Settings material (history, diagnosis, not reproduced)

No Midget in this round. From the code: foreign toplevels are never added to the presenter stack (`set_window_stack`
takes only the shell's windows) and the shell never paints a foreign window, so no shell path paints Files black.
Remaining candidates: (a) the shell believes it is key while the compositor activated Files (`State::focused` is the shell's
own front window, never the compositor's activation), and the inactive path of `Presenter::draw` paints
`Kind::inactive()` plates, near-black in Dark (`MATERIAL_SIDEBAR_SOLID` 20,20,22); (b) Files' own alpha dma-buf
composed black by the compositor. Check on Midget with a screenshot while Files is open, Dark and Light, with and without
the compositor.

### Migration plan: one window model for built-in and external apps

1. (done) Shell windows request blur through ext-background-effect when the server offers it.
2. Move the region computation into `springboard/ui` (a `material` module), re-exported by `bifrost_app::controls`,
   so the shell and SDK apps share one call (the shell cannot import `bifrost_app`).
3. (done for hosted apps) SDK: `bifrost_app` windows send the blur region from the kit's `source_list` / `toolbar` material rects and keep
   their buffer translucent there. Files then blurs like Settings.
   Status: SDK apps hosted by the shell already declare their materials (`ToShell::Materials`, kit `source_list` /
   `toolbar_band`); `Shell::take_wayland_materials` turns them into window px (`expand_material`, unit-tested) and
   `Wayland::window_materials` sends them as the ext-background-effect blur region of the External(N) window, the
   same path as Settings. No protocol change was needed. Direct apps that own a Wayland surface (route `Direct`)
   do not send a region yet. Unverified on Midget.
4. Server: (plate done, 2026-10-19: Vulkan and GLES paths) draw the inactive-material plate in the window server (it
   knows activation) and send xdg `activated` to shell windows, so foreign focus no longer leaves the shell believing
   it is key.
5. Shell chrome (menu bar, dock, panels) are layer clients (`bifrost-chrome`): give them blur regions too and drop
   `Presenter::set_window_stack` / `under`.
6. (done) labwc is removed and the shell-side live backdrop is deleted (sprint 6).

## Materials (2026-10-12)

The window server composes all windows and backs every material, for built-in and external clients alike, the way
the reference WindowServer does. Code: `wm/src/material_kind.rs` (recipes, pure), `wm/src/bifrost_material.rs`
(protocol), `wm/src/render_server/scene_build.rs` (`material_nodes`), `render_server/material.rs` (blur passes),
`wm/src/glass.rs` and `scene.rs` (GLES path).

### `bifrost_material_v1` (`wm/protocol/bifrost-material-v1.xml`)

`ext-background-effect-v1` has no material kind, so BifrOSt adds a private protocol. `bifrost_material_manager_v1.get_material(surface)`
gives a `bifrost_material_surface_v1` (one per surface). Double-buffered, applied on `wl_surface.commit`:

- `clear()` drops all regions; `destroy` removes them from the next commit.
- `add_region(region, kind, state, blending, emphasized, radius_tl, radius_tr, radius_br, radius_bl)`: at most 16 per surface.
  `kind`: titlebar, menu, popover, sidebar, header_view, sheet, window_background, hud_window, full_screen_ui, tool_tip,
  content_background, under_window_background, under_page_background, glass_regular, glass_clear, selection. `state`: follows_window,
  active, inactive. `blending`: behind_window, within_window.
- With any material region, the surface's `ext-background-effect-v1` blur region is ignored; without, the blur region
  is a `Plain` material (a white veil at 25 %, never replaced by a plate).

### One model (2026-10-12)

The shared model is `bifrost_ui::visual_effect` (`MaterialKind`, 14 kinds, plus `GlassKind` for Liquid Glass). The protocol
values are fixed by `MaterialKind::window_server_wire` (ui) and `wm::material_kind::MaterialKind::from_model` (exhaustive
match; `bifrost-wm` depends on `bifrost-ui`). The 16 protocol values are the 14 model kinds plus `glass_regular` (13) and
`glass_clear` (14); a table test in `material_kind.rs` fails if a kind exists on one side only. The shell reaches the
protocol from `src/bare/wayland.rs` (`material_client.rs`): when the compositor offers `bifrost_material_manager_v1` each
window's regions are sent with `add_region` (kind, state, blending, emphasized, four radii; built-in windows and external
apps alike, from `glass::Rect::detail`); otherwise it falls back to `ext-background-effect` blur regions. The shell/app wire
carries the same fields (`proto::Material`, SDK protocol 2).

### Recipe and rules

Each kind has a `Recipe` (blur radius, saturation, Dark and Light tint, opaque inactive plate, noise, always-active).
Tints equal the shell's `MATERIAL_*` pairs (`render/src/ui.rs`).

- Behind-window: samples everything composited below the window (other clients, layers, wallpaper). The compose pass blurs
  the target so far, so this is exact on the Vulkan path.
- Active state: a `follows_window` region in a window that is not key draws the opaque inactive plate, except kinds that
  follow no window state (menu, popover, sheet, HUD, tooltip, full-screen UI, plain). An explicit `active` or `inactive` wins.
  The client leaves a behind-window region clear (a faint tint at most) and the window server backs it, so a region
  that resolves to a plate must really get one: the Vulkan path draws it as a node (`material_nodes`), the GLES path
  as a rounded pixel-shader element under the surface (`scene::backing_of`, `glass::Glass::plates`, shader `PLATE`,
  corner radius from the material). Until 2026-10-19 the GLES path skipped plates ("the client paints those itself",
  which no client does), so an inactive window's sidebar and toolbar were see-through: its text printed over the
  windows behind it and the wallpaper showed through (E2E S1, `docs/e2e/2026-10-19-finder-browser.md`). A plate is
  always opaque (`material_kind::recipe` tests, `scene::tests`); an inactive window dims only its chrome and its body
  (`desk::BACKGROUND_DIM`, which keeps alpha) and never loses opacity. If the plate shader fails to compile the log
  says `bifrost-wm: plate shader: ...` and the plates are left out.
- Within-window: not isolated yet (the compose pass cannot sample one window's earlier pixels alone); resolves to the flat
  plate of the kind. A gap, recorded in the parity rows.
- Rounded mask: the largest corner radius masks a single-rectangle region with the compose shader's anti-aliased SDF.
  Per-corner radii need a shader change (not done).
- Emphasized: stronger tint (+10 % alpha) and saturation (+0.2).
- Damage: unchanged. The Vulkan path blurs inside the frame damage; the GLES path re-blurs only when the signature of the
  elements below (id, commit, geometry), the output size or the recipe saturation changes.
- Budget: `budgeted_blur` lowers the number of dual-Kawase passes until the pyramid (levels written down and up) is at most
  3.5 M texels, about 2 ms on a Skylake HD 530; a full 2560x1440 region costs about 2.5 M texels at 5 or 6 passes, so the budget only bites on larger outputs or radii.
- Colour stages: saturation, brightness and luminance mapping per recipe, as pure math in `wm/src/material_filter.rs`. The compose shader and the GLES glass path apply the same arithmetic. Measured values: `docs/reference/materials-recipes.md`.
- Noise is in the recipe table but not drawn yet; the GLES path leaves the tint to the client.
- Appearance comes from `WmState::material_dark` (`BIFROST_WM_APPEARANCE=light` for Light); no appearance protocol yet.

## Accessibility display options (for the window server)

The shell writes `/run/bifrost/user/accessibility` (atomic rename) whenever Settings > Accessibility > Display changes
Reduce Transparency, Increase Contrast or Button Shapes. Format: `key=0|1` lines for `reduce_transparency`,
`increase_contrast`, `differentiate_without_color`. Increase Contrast implies Reduce Transparency (the reader applies it).
A missing file means all off. Read it with `bifrost_render::appearance::DisplayOptions::read_from(DISPLAY_FILE)` (poll
it, or watch the directory with inotify) and turn glass off (opaque material tints, no blur) while `reduce_transparency` is set.

## linux-dmabuf feedback and the render node (2026-10-19)

The DRM backend advertises `zwp_linux_dmabuf_v1` as a version 4 global with default feedback
(`DmabufState::create_global_with_default_feedback`). Before, it created a version 3 global (`create_global`), which
carries formats but no device. Mesa's Wayland EGL platform learns the GPU from `main_device` in the feedback (or from
the legacy `wl_drm` global, which Smithay 0.7 does not provide), so a sandboxed client that opened an EGL display
(the Browser engine's hidden winit window) logged `MESA-EGL: warning: failed to get driver name for fd -1` and
`MESA-LOADER: failed to retrieve device information`, then fell back to software rendering.

Now `drm::run` derives the render node from the card it opened (`/dev/dri/card1` gives `/dev/dri/renderD128`, via
`DrmNode::from_path(..).node_with_type(NodeType::Render)` in `sync::render_node_dev`; a card without a render node
reports its own number). `sync::plan_from_formats` builds one main tranche on that device from the renderer's dma-buf
texture formats, and `sync::build_feedback` turns it into Smithay's `DmabufFeedback`. Per-surface feedback
(`get_surface_feedback`) answers with the same default; there is no scanout tranche, because clients render for the
compositor's GLES renderer and the compositor composes. If the render node cannot be found, the log says
`dmabuf feedback unavailable` and the v3 global is created as before.

Verify: the window server logs `bifrost-wm: dmabuf feedback v4, main device 226:128`; a client run with
`WAYLAND_DEBUG=1` shows `zwp_linux_dmabuf_feedback_v1.main_device` followed by `tranche_target_device` and
`tranche_formats`; the Browser engine logs `renderer=gpu`. Host test: `sync::tests::default_feedback_names_the_render_node_as_main_device`.

## Key bindings

`wm/src/keybindings.rs` takes these chords before any client sees them (the shell only gets keys while one of its
surfaces has focus). Modifiers must match exactly, so Super+Shift+Space is not the search panel. Super+A is not bound: Cmd+A is
Select All and goes to the app (until 2026-10-20 it started the `fuzzel` app menu, which swallowed Cmd+A in every text field). Because
a chord the window server takes never reaches the shell, the shell's modifier tracking sees only the Cmd press and release; on the
desktop a lone Cmd does nothing (`HwKeys::set_desktop`), so such a chord no longer sends the windows to the Dock.

| Chord | Action |
|---|---|
| Super+Space | sends `quicksearch` to the shell control port (127.0.0.1:7777) |
| Ctrl+Up, F3 | sends `overview` to the shell control port (Mission Control opens or closes) |
| F4 | sends `appgrid` to the shell control port (the Apps grid opens or closes) |
| Alt+Tab | focuses the lowest shown window, so repeated presses walk the stack |
| Alt+F4 | closes the focused toplevel |
| Ctrl+Super+Left / Right | tiles to a half |
| Ctrl+Super+Up / Down | maximizes / centres at the previous size |

## Window frames reported to the shell (2026-10-19)

The window server places, moves and resizes toplevels (cascade at map, `xdg_toplevel.move` and resize grabs, tiling,
maximize), and xdg-shell never tells a client where its window is. The shell kept a rectangle of its own for every
window (`Desk::wins[].r`, with its own cascade), so the two drifted: on Midget `tools/midget.py state` printed
`window=External(9),852,339,...` for Files drawn at 800,360 (N1 of the Files and Browser E2E re-test, 2026-10-19, in `docs/e2e/`), and an
earlier run was off by another amount. Everything the shell puts on the screen from that rectangle landed off by the
difference: app context menus (`ToShell::ContextMenu`, opened at the pointer plus the offset), the share menu
(`share_begin`) and the control socket's `tap`. The pointer itself was fine, because the shell adds its own offset to
surface-local coordinates, which is why only the things placed on the screen were off.

One source of truth: the window server. `bifrost_window_level_v1` version 2 adds the event
`bifrost_window_level_manager_v1.frame(surface, x, y, width, height)` (`wm/protocol/bifrost-window-level-v1.xml`):

- The frame is `wm`'s rectangle of the toplevel, the window geometry the client set (a shadow margin outside it is not
  part of the frame), in logical desktop px.
- Server (`wm/src/window_level.rs`, `FrameReports`; `WmState::report_frames` in `state.rs`): after every `place` and
  `apply_frames` (map, interactive move, tiling keys, maximize, restore, area change) and when a client binds, it
  sends each bound manager the frames that changed since that manager was last told (`changed_frames`), only for
  toplevels of the manager's own client. A minimized or gone toplevel is forgotten, so it is reported again when shown.
- Client (`src/bare/wayland.rs`): the shell binds version 2 (`window_level_client::VERSION`), maps the surface to its
  window id and queues `WindowEvent::Frame(id, x, y, w, h)`.
- Shell (`desk.rs`, `wayland_window_event`, `frame_follows`): the position of `wins[i].r` becomes the reported one and the
  frame is republished (`dirty`). The size stays with the configure (`WindowEvent::Resize`). Context menus, the share
  menu and `tap` then use the real origin with no further change. A window the shell opened but the window server has
  not mapped yet keeps the shell's cascade position until the first report.

Not done: the shell's own title-bar drag and tiling (`Desk::move_to`, `tile`) only run without a window server.
The on-screen-keyboard lift (`desk_keyboard_lift`) still moves `wins[i].r` without moving the toplevel, so the two
differ while the keyboard is up (tablet); the window server has no request to move a client's toplevel.

## Chrome popups with room for their shadow (2026-10-19)

A shell popup (a bar menu, an app's context menu, the Dock's menu, an open or save panel) is chrome surface id 1,
exported by `Shell::desktop_chrome_frames` from the desktop frame. It used to be cropped exactly to the menu, so the
shadow `draw_desktop_popup` draws around it was cut off and a menu over white content had no visible edge (E2E N6).
Now the surface is the menu grown by its shadow (`popup_surface_rect`: `popup_damage_rect` clipped to the screen) and
its input region is the menu rectangle (`ChromeFrame::input`, set with `wl_surface.set_input_region` in
`Display::chrome`), so a click on the shadow still reaches the window below and closes the menu; the shell's hit test
is still `desktop_popup_rect`. `draw_desktop_popup` draws `soft_shadow` and a one-pixel edge in the separator colour
(`ui::separator_now`) on every path, including the CPU path, which also rounds its blurred background to the corners.

## Window models

Pure Rust modules without Smithay types, host-tested. Most are models the compositor does not drive yet:

- `tiling.rs`: half and quarter frames, edge and corner drop zones for drags, arrow-key layouts (Fn+Ctrl+arrows).
- `spaces.rs`: desktops and a space per full-screen window (Ctrl+Left/Right step, Ctrl+1 to 9 pick).
- `overview.rs`: all windows as a thumbnail grid, desktops in a strip, and the per-app overview.
- `stage.rs`: the focus strip, the active window large with recent ones as thumbnails.
- `output_layout.rs`: arrangement and mirroring behind Settings > Displays. The DRM backend drives one connector, so layouts are not applied to hardware yet.

## Safe mode

`/usr/lib/bifrost/session` restarts `bifrost-wm` in safe mode after three unplanned exits within 60 s
(`/run/bifrost/compositor-fallback`, exit times in `/run/bifrost/wm-safe-exits`). Safe mode uses the default GLES renderer,
no direct apps and no chrome clients (`session-policy` prints no environment words). Three more exits in safe mode end the
session and `bifrost-update-guard` (the recovery flow) takes over. Safe mode lasts until the next boot.

## Screen capture

`wlr-screencopy` (`grim`; the screenshot tools write the file through the same hook)  and `ext-image-copy-capture` ask the
backend through `SessionHooks::capture(&mut WmState, &CaptureSpec) -> Option<Vec<u8>>` (`session_protocols.rs`). The
hook returns RGBA rows (R, G, B, A in memory) of the requested region; `write_shm` converts them to the client's
XRGB8888 or ARGB8888 buffer. Without a hook the request is answered `failed`.

- The DRM and headless backends set the hook (`wm/src/capture.rs`, `capture_scene`). A capture renders the scene again
  with `scene::build` (the element list the output uses: wallpaper layer, windows, layer surfaces, glass) into an
  Abgr8888 `GlesTexture` through `OutputDamageTracker`, then `ExportMem::copy_framebuffer` reads the region. Abgr8888
  is R, G, B, A in memory, so the order needs no swap. This runs only for a pending request, never per frame.
- Region and size are in the displayed orientation: `capture_size` turns the mode by the output transform, and the
  capture texture is rendered in that orientation (output transform Normal), so rotated outputs need no row shuffle.
  Regions are clipped to it with `clamp_region`.
- `overlay_cursor` paints the pointer on DRM; the headless output has no pointer image. ext-image-copy-capture cursor
  options are not read yet.
- The Vulkan scanout (`BIFROST_WM_RENDERER=vulkan`) has no readback, so captures fail there.
- Tests: `capture_size`/`clamp_region` unit tests, and `tests/capture.rs` (headless, red scene reads back `[255,0,0,255]`).

## Text input, input method and virtual keyboard

`bifrost-wm` serves three protocols so an on-screen keyboard can type into any Wayland application (`wm/src/text_input.rs`;
Smithay relays the messages, the module adds the policy, the candidate popup and the wiring).

| Protocol | Bound by | Offered to |
|---|---|---|
| `zwp_text_input_v3` | applications (toolkits do this for their text fields) | every client, when the policy below is not `off` |
| `zwp_input_method_v2` | the keyboard or IME process, one at a time | clients of the compositor's user |
| `zwp_virtual_keyboard_v1` | a process that sends key events without a device (keys for applications with no text input, such as X11 clients) | clients of the compositor's user |

What the compositor does:

- **Show and hide.** The text input follows keyboard focus. When the focused client sends `enable` and `commit`, the input method
  gets `activate`, then the surrounding text, the change cause, the content type (hint and purpose) and `done`; the keyboard
  shows itself. `disable`, a focus change (`leave`) and a client that goes away give `deactivate`, and the keyboard hides.
  `WmState::text_input_active` answers the same question for the compositor.
- **Text back to the application.** `commit_string`, `set_preedit_string` and `delete_surrounding_text` reach the focused
  client's text input; the input method's `commit(serial)` closes the batch (a stale serial makes the client discard it).
- **Candidate window.** An input method may give one surface the popup role. It is placed below the text cursor rectangle the
  client reports, flipped above it when it would leave the output, kept inside the output horizontally (`place_popup`), drawn
  above the windows on both render paths, hit-tested before the windows (pointer and touch can pick a candidate) and given
  frame callbacks. A docked keyboard needs none of this and just does not create a popup.
- **Who may bind.** An input method grabs the keyboard and reads the text around every cursor; a virtual keyboard types into
  any window. Both globals are hidden from clients of another user, and from every client when the credentials cannot be
  read. `BIFROST_WM_INPUT_METHOD=off` removes all three globals. The sandbox proxy (`docs/wayland-filter-design.md`) does not
  list the two privileged globals for native apps.
- **Limits.** There is one input method per seat, so the shell must be the only binder: Smithay answers a second bind with `unavailable` on the object that is already registered and does not replace it. Text input exists for Wayland clients
  only: X11 applications get keys from the virtual keyboard, not text. The window server's chords (`keybindings.rs`) are
  intercepted before the keys reach an input method's keyboard grab, so a keyboard that grabs the keys never sees them.

Binding the on-screen keyboard (`docs/keyboard-system-app.md`), planned and not written yet: the keyboard app is an SDK app,
not a Wayland client, so the shell (`src/bare/wayland.rs`, the shell's own connection to `bifrost-wm`) is to bind
`zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1` once and translate, using the mapping already written in
`src/shell/textinput.rs`:

| Input method event | Shell action |
|---|---|
| `activate` + `done` | show the keyboard (`ToApp::KeyboardConfig` with `SHOWN`) |
| `deactivate` + `done` | hide it |
| `surrounding_text`, `text_change_cause` | `TextState` for predictions and auto-capitals |
| `content_type` (hint, purpose) | `InputTraits` (layout, return key, secure entry, auto-correct) |
| `KeyboardOut::Text`, `Composition::Preedit` | `commit_string`, `set_preedit_string` then `commit(serial)` |
| `KeyboardOut::Backspace` | `delete_surrounding_text(1 char, 0)` or a virtual-keyboard backspace |
| `KeyboardOut::Enter` and arrows | virtual-keyboard keys (no input-method request carries them) |
| popup surface (candidates) | `get_input_popup_surface`, placed by the compositor |

The shell is to count `done` events and send that number as `serial` in `commit`.
