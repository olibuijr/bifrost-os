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
| Keys | Super+Space (search panel via control port), Super+A (apps), Alt+Tab, Alt+F4, Ctrl+Super+arrows | done: `wm/src/keybindings.rs` (see Key bindings) |
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
- **Black desktop after unlock** (fixed in `cbd807d`): the lock unmapped the desktop layer surface and the unlock
  attached a buffer before the new configure.

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
4. Server: draw the inactive-material plate in the window server (it knows activation) and send xdg `activated` to
   shell windows, so foreign focus no longer leaves the shell believing it is key.
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

## Key bindings

`wm/src/keybindings.rs` takes these chords before any client sees them (the shell only gets keys while one of its
surfaces has focus). Modifiers must match exactly, so Super+Shift+Space is not the search panel.

| Chord | Action |
|---|---|
| Super+Space | sends `quicksearch` to the shell control port (127.0.0.1:7777) |
| Super+A | starts `/usr/bin/bifrost-apps` |
| Alt+Tab | focuses the lowest shown window, so repeated presses walk the stack |
| Alt+F4 | closes the focused toplevel |
| Ctrl+Super+Left / Right | tiles to a half |
| Ctrl+Super+Up / Down | maximizes / centres at the previous size |

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
