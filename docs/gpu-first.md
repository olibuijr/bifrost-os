# GPU-first BifrOSt desktop (Wayland)

## Compositor review (round 58)

Findings from reading the presenter end to end (not yet measured on Midget; the lead builds after merging).

**What was static.** Glass blurred only the wallpaper: one dual-Kawase pyramid computed per wallpaper
change, sampled by every glass shape (bar, dock, menus, window sidebars). Windows, other panels and anything
moving behind a material never showed through, so materials looked like tinted wallpaper. Window pixels were
only re-uploaded where damaged (good), but nothing behind a panel reacted to a change.

**Damage.** Per surface a `Damage` of the CPU content is uploaded with `tex_sub_image_2d`; each drawable of the
ring owes the union of what it missed and redraws only those areas inside a scissor. This is correct and cheap.
Gap: a change that is not CPU damage (a glass backdrop changing, a transform) had no way to mark a drawable
owed; `set_shadow` was the only such path.

**Regions.** Windows get `set_window_geometry` and an input region on the content rect; the opaque region is
not set (every drawable is ARGB), so the compositor blends every window. Chrome surfaces set no regions.

**Why black or dark slabs appeared.**
1. Window shadow fill: the shadow quad is a rounded-rect SDF that is also opaque (about 0.46 total alpha of
   black) *inside* the content rect, and it was drawn after the glass and before the content. Under translucent
   window pixels (sidebar and toolbar materials) the blur was darkened into a slab. Fixed: the shadow shader
   masks out the true content rect (`u_hole`), the mask follows the shifted shadow layers.
2. Stale glass shapes: `glass::merge` keeps a stored shape unless it lies wholly inside redrawn damage. A group
   plate (menu bar hover/active) that disappears while the damage only partly covers it keeps its blur quad
   with no CPU tint above it: a blurred-wallpaper slab, dark on dark wallpapers. Not changed here because the
   fix (drop on any intersection) is only safe if every glass draw is recorded even when clipped; recommended
   follow-up: record glass before the clip test in `Surface::glass_rrect`, then switch `merge` to
   intersection and flip the `partly_in_the_damage` test.
3. Surfaces whose content rect is smaller than the drawable (framed windows): the drawable is cleared to
   transparent and only the content and shadow are drawn, so no slab comes from there once (1) is fixed.

**Live materials (new).** The presenter keeps the windows' content textures, so it can compose the real
backdrop: `Presenter::set_window_stack` (z order and content origins, fed from `Wayland::windows`) lets
`rebuild_live` render wallpaper plus every window content into a 1/4-size base, then a dual-Kawase pyramid
(3 down, 2 up) whose last texture is what bar, dock and menus sample. It rebuilds at most once per change
(`live_dirty`, set by window damage, moves, stack changes and effects), only when a shell surface that owes
glass draws. After a rebuild `take_live_changed` makes the shell surfaces owe their glass rects
(`owe_rects`, no content upload) and `take_flush` lets `present` run with empty content damage. Saturation
boost is the existing glass shader (`bifrost_render::glass::VIBRANCY`); tint stays on the CPU content. Window
sidebar vibrancy still samples the static wallpaper blur (a window must not blur itself). Without windows,
or with `set_live(false)`, the static pyramid is used, so Intel software paths work unchanged. Reduce
Transparency already turns glass off at the source.

**Window effects (new).** `Presenter::set_fx(id, Fx)` applies scale, alpha, a shift and a shadow factor to a
window's content inside its fixed drawable; `shell/window_fx.rs` computes `Fx` for open (0.9 to 1 plus fade),
close, minimize (scale to 0.12 and slide toward the dock icon, clamped to the shadow margin) and restore, zoom
(a settle pulse), timed by `motion::Kind` (`render/src/motion.rs`). Limits: the drawable is fixed, so scale
stays at or below 1 and the slide is bounded by the shadow margin; a true dock-warp to the dock icon needs a
temporary larger drawable or compositor-side transforms (a follow-up: render the effect into a screen-size
overlay surface). The shell now starts open, minimize, restore and zoom effects (`Shell::desk_fx_start`); close is the integration
point for the shell track. Inactive windows keep the half-depth shadow.

Goal: every BifrOSt surface is drawn and composed on the GPU, Wayland-native, with Apple-level
materials (Liquid Glass) and no CPU copies between the shell and the compositor. Intel (Mesa iris,
HD 530 on Midget) is the target today; the code is vendor neutral (GBM + EGL + dma-buf) so AMD
(radeonsi) and NVIDIA (GBM backend, driver 495+) slot in. 
## Architecture

| Layer | Today | Code |
|---|---|---|
| Device | EGL on the GBM platform of the render node, GLES 3.0 context, surfaceless | `src/gpu/desktop.rs` |
| Drawables | GBM buffer objects (linear by default, `BIFROST_GPU_TILED=1` for driver tiling + modifier), imported as EGL images = render targets | `Gbm::alloc`, `Presenter::create_surface` |
| Presentation | the same dma-bufs wrapped in `wl_buffer`s through `zwp_linux_dmabuf_v1` (v3, format/modifier checked before use) | `src/bare/wayland.rs` (`gpu_buffers`) |
| Layers | Core Animation model: the CPU raster is the content layer, uploaded only where damaged; the GPU draws wallpaper, glass and content | `src/gpu/wl.rs` |
| Materials | dual-Kawase blur pyramid of the wallpaper, computed once per wallpaper change; glass shapes sample it through a rounded-rect SDF | `Presenter::set_wallpaper`, `bifrost_render::glass` |
| Sync | Mesa: implicit dma-buf fencing (`glFlush`); other drivers wait on an EGL fence (`BIFROST_GPU_WAIT=1` forces the wait) | `Presenter::finish` |

API shape follows Metal (device, drawables, one render pass per present) so a Vulkan backend can
replace GLES behind `Presenter` without touching callers.

Surfaces on the GPU: the desktop background layer and the lock surface (`Backdrop::Full`), the menu
bar, dock and menus (`Backdrop::GlassOnly`), built-in app windows (`Backdrop::None`, 0.4.57). Windows
get a margin of `title_height` px around the content (`create_framed`), `xdg_surface.set_window_geometry`
on the content, an input region on the content only, and a two-layer GPU shadow (`Shadow`, deeper
when focused). Pointer coordinates subtract the margin (`Window::margin`). Maximized windows: no margin.
Presenter keys: background `u32::MAX`, chrome ids 0..2, windows `0x1000_0000 + id`.
Glass shader (0.5.25): saturation per scheme (`u_sat`, `glass::saturation`: 1.8 light, 1.7 dark), a specular
rim lit from the top left with a weaker opposite edge (`u_rim`, normal from the SDF's screen derivatives, as
Kyant0/AndroidLiquidGlass's highlight), drawn only for `glass::Kind::{Chrome, Menu}`, and an interleaved-gradient
dither (+-0.5/255) after the blur against 8-bit banding (KWin, Hyprland and scenefx add zero-centred or positive noise
the same way). Mode 4 fills an inactive window's material with its opaque colour (`.followsWindowActiveState`).
The front window's behind-window materials sample `Presenter::under`: the live backdrop without the front window, so
a sidebar blurs the windows behind it; window contents are sampled with GL_LINEAR into the quarter-size base (no
shimmer through glass).

Glass: `Surface::glass_rrect` and the dock/bar/menu code call `bifrost_render::glass::push` when
`glass::gpu()` is on and draw only the tint; the presenter draws the blur under each shape. The glass
list is replaced on full-damage frames and accumulated otherwise. `glass::set_clarity` (0 clear,
1 tinted) scales tints for the Golden Gate Liquid Glass setting (Settings UI not built yet).

Window vibrancy (`NSVisualEffectView` behind-window blending, .sidebar / .titlebar): windows may carry material
rects in window px (`Wayland::window_materials(id, Vec<Glass>)`); the presenter draws such a window over the blurred
wallpaper at the window origin, so the blur shows through translucent (premultiplied) window pixels and nowhere
else. `Wayland::set_scale(ui.pt(1.0))` sets points to px for the window metrics; `src/bin/shell.rs` calls it when
the scale changes and calls `wayland_window_materials(shell.take_wayland_materials())` before `wayland_windows`.
Sources: an SDK app's `ToShell::Materials` (window points, sent by `widgets::source_list` / `toolbar_band` when
they change) and the Settings sidebar column. The shell grants `appearance::VIBRANCY` only when `glass::gpu()` is
on and Reduce Transparency is off; on the SHM path apps never paint translucent pixels. Each rect takes the window
radius and grows by it on its inner sides (the extra blur lies under opaque pixels), so the blur never leaks past
the rounded window corner. Window pixels keep their alpha end to end: the app writes the tint with `fill_rect`, the
host `blit`, `copy_rect` and the window cache copy pixels, `dim_rect` keeps alpha. Everything outside the
material areas is opaque.

Desktop damage: with the GPU drawing the wallpaper, a desktop frame damages only the menu bar, an
open menu and the dock (plus its hover label margin), unioned with the previous frame's regions
(`Shell::desk_regions_now`). Any other screen or `force_full_redraw` falls back to full damage.

Fallback: if the presenter cannot open (no GBM, no dma-buf import, format not listed by the
compositor) the shell logs `gpu: presenter unavailable, SHM path: ...` and uses SHM. `BIFROST_GPU=0`
forces SHM. In Wayland mode the old `GpuComp` is disabled so only one EGL context lives on the thread.

## Measurements (Midget, 2560x1440, HD 530)

| Path | Desktop frame present | Damage |
|---|---|---|
| SHM, 0.4.55 | 1.9 ms copy (+ compositor upload) | 100 % every redraw |
| GPU presenter, first cut | 15 ms (full upload + synchronous fence) | 100 % |
| GPU presenter, 0.4.56 | 0.47 ms (upload 0.31 + render 0.16), 549 kpx uploaded | ~15 % |
| GPU presenter, 0.4.57 | 0.40 ms (0.25 + 0.15); CPU draw 1.4 ms (was 5-19 ms) | ~15 % |

0.4.57 clears and redraws only the region rects on the CPU (`Shell::desk_clear`). The `cpu draw`
average in the journal line includes built-in app windows (Settings draws in the shell): 1.4 ms with
only an external app open, 9.2 ms with Settings open (`tools/bench.py --midget`, 2026-10-09). Record on-device numbers with `tools/bench.py --midget` (parses the
`gpu present` journal line the shell prints every 10 s) into `docs/perf-history.jsonl`.

## Roadmap (next iterations)

1. Done 0.4.57: region-only CPU draw; app windows on dma-buf with GPU shadows.
2. Rounded window corners in the shader (CPU still clips them); shadows on menus and the dock.
3. Content layers mapped straight into GPU memory (linear dma-buf mmap + `DMA_BUF_IOCTL_SYNC`), no glTexSubImage.
4. Text and icon atlases on the GPU; per-layer transforms for window open/minimise/Mission Control animations.
5. Done 0.4.57: Liquid Glass slider (Settings › Displays, pref `liquid_glass`), vibrancy and rim. Next: clarity also scales blur strength.
6. Explicit sync (`wp_linux_drm_syncobj_v1`) for NVIDIA; format modifiers negotiated through dma-buf feedback (v4).
7. Vulkan backend behind the same presenter API.
8. Own compositor (BifrOSt WindowServer) so glass can blur other clients' windows, which a Wayland client cannot see.

## Debugging

- `journalctl -u springboard | grep gpu` shows the renderer line and the 10 s averages.
- Black screen with the presenter on: another EGL context took the thread (`Presenter::current` guards each entry point).
- Quick device test without a release: `tools/midget-push-shell.sh`; the next `bifrost pc midget update` restores the packaged binary.

## Idle frames and partial uploads (Snow Leopard round 2)

- Cause of presents on an unchanged desktop: `Shell::set_info` set `dirty` on every `Info` difference, and `Info`
  carries fields that tick on their own (uptime, free storage, memory text, battery history, media counts, sync
  text, brightness). Each second-scale change drew the bar plus the dock box (prev + current regions, about 13%
  of the screen, 481 kpx) and presented it. Fixed: `shell::ambient_only` keeps those changes from dirtying a
  desktop that shows no page, menu or panel listing them.
- Desktop damage is now exact: the pixels under the bar/dock/menu regions are snapshotted before `draw_desk` and
  compared after (`Damage::snapshot_rects` / `Damage::changed_in`, render/src/gfx.rs). Identical pixels give an
  empty damage, and `present` already returns without upload or commit on empty damage. A clock tick now
  damages the changed row runs of the bar only. The GPU presenter uploads exactly the damage areas.
- The cpu-path frame trace line now ends with `why <screen>:<reasons>` (`bifrost pc` trace on).
- Tests: `gfx::tests::changed_in_*`, `shell::idle_tests::ambient_fields_alone_are_not_a_visible_change`.
