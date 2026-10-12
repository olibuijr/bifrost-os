# G2: protocol sync and presentation feedback (`wm-sync-feedback`)

Owner: wave G2 agent `wm-sync-feedback`. Contract: `docs/graphics-architecture.md`, "Window server" and "Frame clock".
Plan row: `docs/graphics-waves.md`, G2 row 4. Code: `springboard/wm/src/sync.rs`, with glue in
`springboard/wm/src/handlers.rs`, `springboard/wm/src/state.rs` and `springboard/wm/src/lib.rs`.

## What is implemented

### Pure logic (`sync.rs`, no Smithay types, host-tested)

- `plan_feedback` / `Tranche` / `FeedbackPlan`: linux-dmabuf v4 feedback planned from the render device's formats
  and the scanout device's formats. Scanout formats are kept only when the renderer can also import them, they come
  first as a `scanout` preference tranche, and the main tranche (render formats) comes last. Duplicates are removed
  in order.
- `surface_plan`: per-surface feedback. A scanout candidate (a full-screen window on the scanout device) keeps the
  scanout tranche; any other surface gets the render-only tranches.
- `Flips`: vblank count and frame targets. A queued frame is shown at the next vblank not already promised to an
  earlier frame.
- `Deferred<T>`: items held until a sequence number is reached, released oldest first. Used for presentation feedback
  (released at the vblank that shows the frame) and for syncobj release points (released when the GPU has finished
  the submission that read the buffer).
- `timestamp` and `presentation_kind`: the `wp_presentation.presented` time split into seconds and nanoseconds, and
  the flags (`Vsync`, `HwClock`).
- `device_number`: `st_rdev` of a device node, the `dev_t` values the feedback needs.

### Smithay glue

- `build_feedback(plan)`: converts the plan to Smithay's `DmabufFeedback` with `DmabufFeedbackBuilder`
  (`add_preference_tranche` with `TrancheFlags::Scanout` for the scanout tranche).
- `dmabuf_global(state, plan)`: creates the linux-dmabuf global with version 4 default feedback
  (`create_global_with_default_feedback`).
- `FrameFeedback` plus `queue_frame(state, output)` and `on_vblank(state, time, refresh, vsync)`: each queued frame
  takes the presentation feedback of every window and layer drawn in it (Smithay's
  `take_presentation_feedback`). At the vblank that flips it, the feedback is sent with `presented` using the
  monotonic vblank time, `Refresh::fixed(refresh)` (or `Unknown` at zero) and the flip flags.
- `watch_surface(surface)`: adds a pre-commit hook to every surface, from `CompositorHandler::new_surface`.
- `explicit` (behind the `drm` feature):
  - `install(dh, render_node)`: opens the render node and creates Smithay's `DrmSyncobjState`, only when
    `supports_syncobj_eventfd` is true. Otherwise it returns `None` and the protocol is not offered.
  - `gate_acquire`: the pre-commit hook. A pending acquire point that has not signalled adds a
    `DrmSyncPointBlocker` (`add_blocker`), and a calloop source wakes the client's compositor state when the point
    signals. A commit is held until its buffer may be read.
  - `signal_released(points)`: signals release points.

### Wiring (`handlers.rs`, `state.rs`, `lib.rs`)

- `lib.rs`: `pub mod sync;`.
- `state.rs`: new fields `loop_handle` (the event loop's handle), `presentation` (`wp_presentation` on
  CLOCK_MONOTONIC, clock id 1), `frame_feedback` (`FrameFeedback`), and `drm_syncobj_state` (behind `drm`, `None`
  until the DRM backend installs it).
- `handlers.rs`: `new_surface` calls `sync::watch_surface`; `delegate_presentation!`; with `drm`, the
  `DrmSyncobjHandler` impl and `delegate_drm_syncobj!`.

## Mapping to the contract

| Contract item | Where |
|---|---|
| linux-dmabuf v4 feedback, main device = the render node, tranches from the device formats and the scanout plane formats | `plan_feedback`, `build_feedback`, `dmabuf_global`, `device_number` |
| per-surface feedback | `surface_plan` (the rule). The per-surface hook is not called yet, see todo |
| `wp_linux_drm_syncobj_v1` acquire points gate the commit (the render never samples before the acquire point) | `explicit::gate_acquire`, `watch_surface` |
| release points are signalled when the server is done with the buffer | `Deferred` (the ordering model) and `explicit::signal_released`; see todo for the GPU-completion hookup |
| `wp_presentation` feedback from the frame clock with the real vblank time and refresh | `FrameFeedback`, `queue_frame`, `on_vblank`, `timestamp`, `presentation_kind` |

## Deviations from the task text (`contract_changes`)

- The acquire and release points are not imported as `bifrost_gpu::SharedEvent`. `bifrost-gpu` has no syncobj or
  `SYNC_FD` import API, and `sync.rs` may not add `unsafe`. The acquire gate uses Smithay's CPU-side blocker
  (an eventfd on the calloop), which holds the commit until the point signals. That gives the same ordering for the
  window server's reads, but the GPU queue does not wait on the point itself. Adding `SharedEvent::import_syncobj(fd,
  point)` to `bifrost-gpu` would let the render server wait on it on the GPU. This needs a `bifrost-gpu` owner change.
- The state gains fields (`loop_handle`, `presentation`, `frame_feedback`, `drm_syncobj_state`). The skeleton had none.

## Not done (todo_left)

1. Nothing creates the feedback global yet. `wm/src/drm.rs` still calls
   `state.dmabuf_state.create_global::<WmState>(&state.dh, formats)` (version 3). The hook is one call:
   `let plan = sync::plan_feedback(main_dev, &render_formats, scanout_dev, &scanout_formats);
   state.dmabuf_global = sync::dmabuf_global(state, &plan)?;` where `main_dev = sync::device_number(render_node)?`.
   The file that owns that call (`drm.rs`, `wm-drm-vulkan`) must make it. Nested and headless have no render node,
   so they keep no global.
2. `DmabufHandler::new_surface_feedback` is not implemented. `surface_plan` is ready, and the hook needs the surface's
   output and fullscreen state from `wm.rs`.
3. `sync::install_syncobj` is not called. The DRM backend must call it with the render node, then set
   `state.drm_syncobj_state`. Until then the protocol is not advertised and clients use implicit sync.
4. `queue_frame` and `on_vblank` are not called. `drm.rs` must call `queue_frame(state, output)` after each
   `queue_frame` of the DRM compositor, and `on_vblank(state, time, refresh, vsync)` from the page-flip event, with
   the flip's monotonic time (the `FrameClock` input).
5. Release points are signalled by Smithay when the next buffer replaces the current one, and when the surface is
   destroyed. `Deferred<DrmSyncPoint>` is the model for holding them until the GPU has finished the submission that
   read the buffer. To do this, the render path must take the release point out of the cached state (Smithay's
   `DrmSyncobjCachedState`) and hold it until the submission completes.
6. The acquire gate is CPU-side (see above). A GPU-side wait needs `bifrost-gpu` import (see contract changes).
7. Tests were written but not run here (the lead's rules allow `cargo check` only). See Verify.

## Verify

Host (no GPU, no compositor):

```
cd springboard
CARGO_TARGET_DIR=$PWD/target-agents cargo check --offline -j 2 -p bifrost-wm --tests    # passes (done)
CARGO_TARGET_DIR=$PWD/target-agents cargo test --offline -p bifrost-wm sync::          # pure tests, to run
```

The `sync::tests` cover: tranche planning (dedupe, order, scanout filtered to importable formats, no scanout tranche
without a device or importable format), the surface rule, `build_feedback` for a valid and an invalid fourcc,
`Flips` (targets 1, 2, 3, 4 across two queued frames and three vblanks), `Deferred` (hold order, release up to a
sequence), timestamps (1500 ms is `(1, 500_000_000)`), presentation flags, and `device_number` (a regular file is 0,
`/dev/null` is 259 = `(1 << 8) | 3`).

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4). Requires the DRM backend wiring (todo 1 to 4) and the session set to
`bifrost-wm` (`/etc/bifrost/compositor` says `bifrost-wm`, with `BIFROST_WM_RENDERER=vulkan`):

1. `ssh midget`, then `wayland-info` (from the `wayland-utils` package) inside the session. Expect
   `zwp_linux_dmabuf_v1` version 4 with feedback events, and `wp_presentation` version 2.
2. Run a Vulkan or GL client that uses dma-buf (for example `vkcube --wsi wayland` or a GTK4 app). Expect no
   fallback to shared memory in the client log.
3. For explicit sync, start an app that binds `wp_linux_drm_syncobj_manager_v1` (`wayland-info` lists it when the
   kernel supports syncobj eventfds). Expect no protocol error on commit.
4. For presentation, `BIFROST_WM_STATS=1` plus a client that requests `wp_presentation` feedback (`weston-presentation-shm`
   or a Chromium `--enable-features=WaylandPresentation`). Expect `presented` events with `refresh` about 16.67 ms.
5. `tools/midget-push-shell.sh` and the screenshot check from `docs/bifrost-pc-workflow.md`; the desktop must still
   draw.
