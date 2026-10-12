# G2 `wm-drm-vulkan`: the Vulkan scanout on the DRM backend

Status: the frame loop is wired. With `BIFROST_WM_RENDERER=vulkan` (or `/etc/bifrost/renderer` says `vulkan`) the DRM
backend opens the render server on the scanout GPU, allocates the output's scanout ring with GBM, renders each frame
through `render_server::Server::frame` and presents it with an atomic KMS commit whose IN_FENCE_FD is the submission's
sync_file. Page-flip vblanks drive the buffer ring and `FrameClock`. The GLES scanout stays the default and the
fallback. **The Vulkan frame has no scene yet** (see "What is left"), so on Midget the output shows the clear colour
only. What this slice proves on hardware is the Vulkan render, the explicit-sync commit and the vblank-paced loop.

## What is implemented

`springboard/wm/src/drm_vulkan.rs` (owned here):

- `render_node_name`, `render_node_for_card`, `probe`: the render node of the scanout card (as before).
- `pick_scanout_format(plane, device)`: pure. The first of XRGB8888, then ARGB8888, for which the plane lists a modifier
  that Vulkan also imports (`Device::supported_dmabuf_formats`). The plane's order decides between modifiers.
- `refresh_period(mhz)`: pure. A millihertz refresh becomes the frame period; zero means unknown (`FrameClock` falls
  back to 60 Hz).
- `scene_nodes()`: the scene for a frame. Returns no nodes until the scene builder exists (see "What is left").
- `Pacing`: pure buffer-age bookkeeping. For the buffer a frame renders into, it returns the damage to redraw and the
  load action. A buffer of age `a` (frames since it last held output, `a <= RING`) redraws the damage of the last `a`
  frames with `Load`. Any other buffer (never drawn, forgotten after a failure, or older than the ring) is cleared and
  the whole output is redrawn. A cleared frame is recorded as full damage.
- `start(card, surface, size, refresh_mhz)`: opens the render server, picks the format, allocates `RING` GBM buffer
  objects (`GbmAllocator`, `RENDERING | SCANOUT`, the one modifier chosen), makes KMS framebuffers with
  `framebuffer_from_bo`, and imports them as render targets with `OutputTargets::from_gbm` (the targets get `SAMPLED`
  usage from `Device::import_dmabuf`, so live glass can sample them).
- `Vulkan::frame(surface)`: when no flip is pending and a buffer is free, acquires the slot, plans the damage, calls
  `Server::frame` with the scene, takes the submission's sync_file (none when the GPU already finished, which Smithay
  does not need), and commits the plane: `DrmSurface::commit` while the surface has a pending modeset, otherwise
  `page_flip`, both with `PlaneConfig::fence` set to the sync_file. A failure releases the buffer (`abandon`, `forget`).
- `Vulkan::on_vblank(time)`: the queued buffer is now on screen (`OutputTargets::flipped`, which frees the previous one)
  and the vblank time feeds `FrameClock::vblank`.
- `Vulkan::reset()`: session resume. Nothing is queued and every buffer is rewritten.

`springboard/wm/src/drm.rs` (owned here):

- `run`: after `open_gpu` (which still sets up the GLES compositor, the fallback), when `render_server::selected()` the
  Vulkan scanout starts. A failure logs `bifrost-wm: vulkan scanout unavailable, GLES: ...` and keeps GLES.
- `open_gpu` takes the shared vblank-time cell. The DRM event source writes the page flip's monotonic time (Smithay's
  `EventMetadata`) into it before the vblank callback runs.
- `Drm::render`: with a Vulkan scanout, it calls `Vulkan::frame` and sets `pending` from the result, then
  `send_frames`. Otherwise the GLES path runs unchanged.
- The vblank callback calls `Vulkan::on_vblank` (GLES: `frame_submitted`). The VT resume calls `Vulkan::reset`.

`springboard/wm/src/lib.rs`: `drm_vulkan` is now `#[cfg(feature = "drm")]`, since it uses Smithay's KMS and GBM types.

## Mapping to the contract (`docs/graphics-architecture.md`)

| Contract item | Status |
|---|---|
| Device on the scanout GPU's render node | done (`probe`, the same card as the GLES path) |
| GBM scanout buffers with formats and modifiers the plane accepts, imported as render targets with `SAMPLED` | done (`start`, `OutputTargets::from_gbm`) |
| `Server::frame` into the acquired target with the damage of its buffer age | done (`Pacing`), but the damage is always the full output until the scene reports changes |
| Atomic KMS commit with the submission's sync_file as IN_FENCE_FD | done (`PlaneConfig::fence` through `commit`/`page_flip`) |
| Page-flip events drive the frame clock and the buffer ring | done for the clock's vblank times and the ring (`on_vblank`); frame callbacks are sent after each frame, without a predicted presentation time |
| Frame scheduling at `FrameClock::start_at` | not yet: frames start on the vblank (and on the 16 ms timer while idle), the clock records the times and the frame durations |
| GLES stays the default and the fallback | done (the probe or the start may fail; GLES scanout is set up first in `open_gpu`) |
| Failure paths: `frame` error or EBUSY releases the buffer; VT switch resets | done (`fail`, `reset`) |
| Scene from the window server's builder | not yet (`scene_nodes` returns nothing) |

No contract change. `render_server::scene_build` does not exist in this tree, so `scene_nodes()` is the single place the
scene builder plugs in.

## What is left (todo_left)

1. **Scene.** Replace `drm_vulkan::scene_nodes()` with wm-scene's builder (`render_server::scene_build`, or whatever
   it exports) fed from `WmState`: surface trees, layer-shell surfaces, the cursor, the boot fade. Until then the
   Vulkan output is the clear colour. This is the first thing to merge for a useful Midget result.
2. **Damage.** Replace the full-output frame damage in `Vulkan::frame` with the scene's damage, so `Pacing` keeps
   buffers and redraws only what changed. The buffer-age logic is already in place and tested.
3. **Scheduling.** Compose at `FrameClock::start_at(now)` (a timer, not the vblank) and pass
   `FrameClock::frame_took` the real compose+submit time (done) so the budget adapts.
4. **Presentation feedback and frame callbacks** with the predicted presentation time (`FrameClock::next_presentation`);
   `wm-sync-feedback` owns `wp_presentation`.
5. **Cursor.** The GLES path draws the arrow as a `MemoryRenderBuffer`. The Vulkan path needs the cursor in the scene
   (wm-scene) or a hardware cursor plane.
6. **Boot fade** (`BIFROST_BOOT_FADE`) is GLES-only for now.
7. **Client dma-buf imports** still go to the GLES renderer (`dmabuf_import`). The Vulkan importer needs wl_buffers
   (`render_server::import`, wired by wm-scene and wm-sync-feedback).
8. **Hardware tests.** The Midget run below is manual. A GPU test of `start` with a headless KMS device is not possible
   in CI; the pure parts are unit-tested.

## Tests

`cargo test -p bifrost-wm drm_vulkan` (run by the lead) covers: the render node names (8), the scanout format choice
(5), the refresh period (2), and `Pacing` (age 1, age 2, age up to the ring size, older than the ring, the first frame,
forgotten buffers, steady triple buffering, forget_all). `drm.rs` keeps its mode tests.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

The session runs labwc unless `/etc/bifrost/compositor` says `bifrost-wm`.

1. Ship the build: `bifrost pc publish`, then `bifrost pc midget update --restart`. Check
   `python3 tools/midget.py recovery` says `phase=accepted`, then `bifrost pc midget key enter` to unlock.
2. Select the session and the renderer on Midget:
   `ssh midget 'echo bifrost-wm | sudo tee /etc/bifrost/compositor; echo vulkan | sudo tee /etc/bifrost/renderer'`
   (or `BIFROST_WM_RENDERER=vulkan` in the compositor's unit), then restart the session.
3. Read the log: `ssh midget 'journalctl -b -t bifrost-wm | grep -i vulkan'`. Expect, in order:
   `vulkan scanout WxH Xrgb8888 modifier 0x... , 3 buffers` (or the reason it fell back to GLES), then
   `vulkan scanout: first frame on screen`. A `vulkan frame:` line is a failed frame; the loop retries every 16 ms.
4. Check the output: `python3 tools/midget.py` screen capture (`bifrost pc midget screen /tmp/vk.png`) shows the clear
   colour (0.06, 0.06, 0.08) and no garbage. Until the scene lands the desktop itself is not drawn on this path, so
   this step only proves scanout.
5. Check the flips: the log should show no `kms:` errors while the desktop runs; `vulkan scanout` lines repeating
   means the commit path failed; the log line says which step (`render`, `sync_file` or `kms`).
6. Revert with `echo labwc | sudo tee /etc/bifrost/compositor` and `sudo rm /etc/bifrost/renderer`, restart, and
   confirm the desktop comes back on the GLES path.

## Build

`CARGO_TARGET_DIR=.../springboard/target-agents cargo check --offline -j 2 -p bifrost-wm --tests` passes with no
warnings from `bifrost-wm`. The two `bifrost-gpu` warnings (`entry` unused, two unused sub-allocator methods) were
already there.
