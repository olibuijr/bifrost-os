# BifrOSt graphics architecture (final)

Owner decision 2026-10-12: rebuild the graphics core the way the reference platform does it, as one final
architecture rather than a series of intermediate designs. GPU API: Vulkan. This document is the contract every
component is written against; when code and this document disagree, fix one of them in the same change.

## Why the desktop feels slow today

Measured from the code (0.5.44):

1. An SDK app rasterises its whole window on the CPU (`bifrost_render::Surface`) into a `memfd` buffer.
2. It sends the buffer to the shell over `SOCK_SEQPACKET` (`docs/apps.md`, "Frames").
3. The shell, a single root process with one event loop that also draws the desktop, copies or uploads those pixels
   into its own xdg toplevel for that app (`src/bare/wayland.rs`, `Window`).
4. labwc composites the toplevel.

Every animation, scroll and hover is a full CPU redraw in the app plus a copy in the shell plus a composite, and any
stall in the shell stalls every app. Window moves feel fine because labwc moves the window without the app or the
shell drawing anything. That is the property the final architecture gives to everything.

## The architecture

| Reference platform | BifrOSt | Crate / path |
|---|---|---|
| Metal | `bifrost-gpu`: a small, safe, Metal-shaped API over Vulkan 1.3 (device, textures, buffers, command queues and buffers, render and compute encoders, pipelines, shared events). The only crate with `unsafe` Vulkan calls. | `springboard/gpu` |
| IOSurface | dma-buf backed `Texture`s with DRM format modifiers, importable and exportable across processes, synchronised with explicit fences (`SharedEvent`, sync_file). | `bifrost-gpu::dmabuf` |
| Core Graphics / Core Text drawing | `Canvas`: one drawing vocabulary (rects, rounded rects, strokes, gradients, shadows, images, glyph runs, paths, clips, transforms). `Surface` implements it on the CPU; `DisplayList` records it; `bifrost-2d` executes display lists on the GPU with SDF shapes and a glyph atlas. | `springboard/render/src/canvas.rs`, `display_list.rs`; `springboard/gfx2d` |
| Core Animation (client side) | `bifrost-layers`: a layer tree (frame, anchor, transform, opacity, corner radius, masks, border, shadow, material, contents) changed in atomic transactions, with implicit and explicit animations (basic, keyframe, spring, timing functions, media timing). | `springboard/layers` |
| Core Animation render server | Inside the window server: it holds each client's committed layer tree, evaluates animations for every display frame and composites the result. Animations never wait for the app. | `springboard/wm/src/render_server/` |
| WindowServer | `bifrost-wm` (Smithay for protocols, DRM/KMS, libinput, libseat): the only compositor. Owns window positions, stacking, focus, cursor, input routing, materials (live blur), window shadows, the frame clock and scanout. labwc is removed once Midget acceptance passes. | `springboard/wm` |
| CADisplayLink / CVDisplayLink | Frame clock driven by vblank: the window server schedules composition just before vblank, sends `wp_presentation` feedback and frame callbacks with the predicted presentation time; the SDK exposes a display link. | `wm/src/render_server/clock.rs`, `sdk/src/display_link.rs` |
| AppKit / SwiftUI | The SDK. Views are layer-backed: a scroll is a layer offset animated by the window server; a hover is a property change; a redraw re-records only the damaged layer's display list. | `springboard/sdk` |
| Dock, menu bar, SystemUIServer, Notification Center | Separate privileged clients of the window server (layer-shell + layer trees), split out of the shell. | `springboard/src/shell` (split) |
| System Settings, App Store, Photos | Ordinary app processes, not shell code. | `springboard/apps/*` |
| Event routing | The window server delivers input straight to the focused client. The app↔shell socket keeps control traffic only (menus, intents, services, permissions), never pixels or input. | `wm/src/input.rs`, `sdk/src/wl/` |

### Process model

```
bifrost-wm  (local account, seatd: DRM/KMS + libinput)  -- Vulkan --> display
  |  Wayland: xdg-shell, layer-shell, linux-dmabuf v4, presentation-time, bifrost_layer_v1, ...
  +-- shell clients: menu bar, Dock, Notification Center, Control Center, desktop  (privileged)
  +-- SDK apps: each a Wayland client; GPU-rendered layer contents (dma-buf), layer tree commits
  +-- native Linux apps: plain Wayland (and XWayland)
springboard shell (root) --SOCK_SEQPACKET control--> SDK apps (menus, intents, services; no pixels)
```

## Contracts

### bifrost-gpu (`springboard/gpu`)

Metal-shaped and safe. Vulkan 1.3 core with dynamic rendering and synchronization2; required extensions:
`VK_KHR_external_memory_fd`, `VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier`,
`VK_KHR_external_semaphore_fd`, `VK_EXT_physical_device_drm` (to match the compositor's DRM render node),
`VK_EXT_queue_family_foreign`. Optional: `VK_KHR_push_descriptor`.

- `Device::open(render_node)` picks the physical device whose DRM node matches (or the first discrete/integrated GPU),
  one graphics+compute queue. `DeviceInfo` names the GPU, driver and API version.
- `Texture` (image + memory + view) from a `TextureDescriptor` (size, `PixelFormat`, usage, exportable, allowed
  modifiers); `Device::import_dmabuf(&DmaBuf)`; `Texture::export_dmabuf()`; `Texture::replace_region` uploads CPU
  pixels through a staging buffer and waits (shm clients, tests); the 2D renderer instead batches glyph, image and
  path-mask uploads into one staging buffer per pass and records them with `CommandBuffer::copy_buffer_to_texture`
  (`gfx2d/src/uploads.rs`), moving sampled textures with `CommandBuffer::prepare_sampled` before the pass.
- `Buffer` (vertex, uniform, storage, staging), mapped when host-visible.
- `CommandQueue::command_buffer()`, `CommandBuffer::{render_encoder, compute_encoder, copy_texture, copy_buffer_to_texture, prepare_sampled, wait, signal, commit}`;
  `RenderEncoder::{set_pipeline, set_viewport, set_scissor, set_bytes (push constants, VERTEX|FRAGMENT, capped to the pipeline's declared size), set_vertex_buffer, set_texture,
  set_sampler, draw}`; `ComputeEncoder::{set_pipeline, set_bytes, set_texture, dispatch}`.
- `Submission::{wait, sync_file}`; `SharedEvent` = timeline semaphore with sync_file export (an imported client
  sync_file is waited on through `CommandBuffer::wait_sync_file`); a commit keeps the events it waits on or signals
  alive until the submission completes.
- Shaders are GLSL in `gpu/shaders/`, compiled to SPIR-V by `build.rs` with `glslc` and embedded; `shader(name)`.
- Memory: dedicated allocations for textures, a simple block sub-allocator for small buffers. No allocator crate.
- Every public type is `Send`; `Device` is cheaply clonable (`Arc`).

### Canvas, DisplayList (`springboard/render`)

`canvas::Canvas` is the one drawing vocabulary: `fill_rect`, `fill_rrect`, `stroke_rrect`, `linear_gradient`,
`shadow`, `image`, `glyphs`, `path`, `push_clip`/`pop_clip`, `push_transform`/`pop_transform`, `push_opacity`/
`pop_opacity`, `material`. Coordinates are f32 device pixels; colours are the existing packed `u32` (premultiplied
RGBA8, `gfx::pack`). `Surface` implements `Canvas` by calling its existing CPU methods, so every current drawing call
keeps working. `display_list::DisplayList` implements `Canvas` by recording `Op`s with their bounds (for damage and
culling). Images and glyphs are referenced by stable ids so the GPU side can cache textures.

### bifrost-2d (`springboard/gfx2d`)

Executes a `DisplayList` into a `bifrost-gpu` `Texture` (a layer's backing store):

- Shapes: instanced quads with an SDF fragment shader for rect, rounded rect, stroke, shadow (analytic Gaussian
  approximation), linear gradient; clips as rounded-rect SDF masks; transforms as per-instance affine matrices.
- Text: glyph atlas (`R8` texture, shelf packer) filled by the existing font rasteriser (`render/src/font.rs`) at the
  needed size and subpixel offset, drawn as instanced quads with the text colour.
- Images: texture cache keyed by image id with LRU eviction; paths are rasterised on the CPU into masks (a tessellator exists in `gfx2d/src/paths.rs`, not wired yet; the plan was fill:
  even-odd/non-zero via fan + stencil-free monotone decomposition; stroke: expanded polygons).
- Batching: consecutive ops with the same pipeline are one draw; target is under 50 draws for a typical window.

### bifrost-layers (`springboard/layers`)

Pure Rust, std only, host-tested; shared by the SDK (client) and the window server (render server).

- `Layer` properties: `bounds`, `position`, `anchor_point`, `transform` (3x3 affine plus z-order), `opacity`,
  `hidden`, `corner_radius`, `masks_to_bounds`, `background_color`, `border`, `shadow` (colour, opacity, radius,
  offset, path-free rounded rect), `material` (none or a glass style: blur radius, tint, saturation), `contents`
  (none or a contents id bound to a dma-buf; a solid colour is the background colour), `contents_gravity`, `sublayers` (ordered).
- `Transaction`: create/destroy layers, set properties, insert/remove sublayers, add/remove animations, set the
  root; applied atomically; carries a sequence number. Implicit animation: a transaction may carry a default
  duration and timing function applied to animatable property changes, like an implicit transaction.
- `Animation`: `Basic { key, from, to }`, `Keyframe { key, values, key_times, timing_functions }`, `Spring { key,
  from, to, mass, stiffness, damping, initial_velocity }`, `Group`; media timing (`begin_time`, `duration`, `speed`,
  `time_offset`, `repeat_count`, `autoreverses`, `fill_mode`); timing functions (linear, ease in/out/in-out, cubic
  bezier) reusing `render/src/motion.rs`.
- `Tree::apply(&Transaction)`, `Tree::presentation(t)` (the model tree with every running animation evaluated at
  time `t`), `Tree::take_finished(t)` (animation ids that ended, for `animation_finished` events).
- `codec`: a compact binary encoding of `Transaction` (little endian, versioned) for the protocol.

### `bifrost_layer_v1` (`springboard/layers/protocol/bifrost-layer-v1.xml`)

A private Wayland extension, small on purpose; the transaction body is the `codec` blob:

- `bifrost_layer_manager_v1.get_context(id, wl_surface)` makes a layer context for a surface (an xdg toplevel or a
  layer-shell surface). The context's root layer is drawn as that surface's content.
- `bifrost_layer_context_v1.bind_contents(contents_id: uint, buffer: wl_buffer)` binds a dma-buf `wl_buffer` to a
  contents id (the layer's backing store); rebinding replaces it on the next commit.
- `bifrost_layer_context_v1.commit(transaction: array)` applies one encoded transaction atomically at the next
  frame; with `wp_linux_drm_syncobj` or an acquire fence the buffers are sampled only after their fence signals.
- Events: `presented(seq, tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns)`, `animation_finished(animation_id)`.

### Window server (`springboard/wm`)

Smithay handles protocols, the DRM/KMS backend, libinput and libseat; rendering is our own Vulkan render server
(Smithay's `GlesRenderer` is not used in the final design):

- `render_server/import.rs`: client buffers to `Texture`s: dma-buf import (no copy), shm upload with damage.
- `render_server/output.rs`: scanout buffers are GBM buffer objects imported into Vulkan as render targets and
  presented with an atomic KMS commit (Smithay `DrmSurface`/`GbmBufferedSurface`); direct scanout of a full-screen
  client buffer when its format and modifier allow.
- `render_server/compose.rs`: draws the scene (outputs, layer-shell surfaces, windows in stacking order, each a
  surface tree or a layer tree) with per-output damage tracking.
- `render_server/material.rs`: live glass: dual-Kawase blur in compute shaders of the content behind a material
  region, rounded-rect clip, window shadows drawn by the server.
- `render_server/clock.rs`: per-output frame clock from vblank timestamps; schedules composition at
  `next_vblank - budget` (budget learned from recent frame times, starting at 4 ms); `wp_presentation` feedback;
  frame callbacks carry the predicted presentation time.
- `render_server/layers.rs`: the `bifrost_layer_v1` server: a `bifrost_layers::Tree` per context, transactions
  applied at frame start, `Tree::presentation(t)` composited, events sent.
- `input.rs`: libinput configuration (acceleration, natural scrolling, tap), gestures (pinch, swipe, hold), kinetic
  scrolling, key repeat; input goes straight to the focused surface.
- Session parity with labwc: foreign-toplevel list, screencopy, cursor shapes, xdg-activation, output management,
  idle/power, session lock, keybindings (tiling, Super+Space, Alt+Tab), server-side decorations for third-party
  apps, XWayland.

### SDK (`springboard/sdk`)

- `wl/`: the app is a Wayland client of the window server: an xdg toplevel, `bifrost_layer_v1` context, dma-buf
  backing stores allocated with `bifrost-gpu` and drawn with `bifrost-2d`, `display_link`, input from `wl_seat`.
- The control socket to the shell stays for menus, intents, services and permissions.
- Until the switch, `BIFROST_DIRECT=1` selects the direct path; the shared-memory path remains the fallback when no
  Vulkan device or no `bifrost_layer_v1` global is present.

## Performance budgets (Midget, 2560x1440 at 60 Hz, HD 530)

| Measure | Budget |
|---|---|
| Window server frame (compose + blur, typical desktop) | under 4 ms GPU, under 1 ms CPU |
| Animation frame rate (window open, menu fade, scroll, Dock) | 60 fps with no dropped frames while the app is busy |
| Input to photon (click, key) | under 25 ms |
| App redraw of one changed control | re-record and draw one layer, under 2 ms |
| Scroll | layer offset only, no app redraw until new content is exposed |

Every component adds its timings to `tools/bench.py --midget` and `docs/perf-history.jsonl`.

## Switch-over

Main stays releasable. The new path is selected by `/etc/bifrost/compositor = bifrost-wm` and `BIFROST_DIRECT=1`
until the acceptance list (`docs/wayland-desktop.md`) and the budgets above pass on Midget; then the session starts
`bifrost-wm` only, labwc and the shell's per-app toplevels are removed, and the shared-memory frame path stays only
as a fallback for machines without Vulkan.
