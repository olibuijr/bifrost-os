# G2: SDK direct runtime (`sdk-direct-runtime`)

Wave G2 row 13 of `docs/graphics-waves.md`. Branch `g2/sdk-direct-runtime`. Files: `springboard/sdk/src/runtime.rs`,
`springboard/sdk/src/wl/frame.rs`. Contract: `docs/graphics-architecture.md`, "SDK" and "Canvas, DisplayList".

## Status: partial. The direct path is not entered yet.

The switch, the gate and the frame module are in place. The runtime does not connect as a Wayland client yet, because
the SDK connection cannot bind `bifrost_layer_*` objects: `sdk/src/wl/connection.rs` `State` has no `Dispatch` impls
for `bifrost_layer_manager_v1` or `bifrost_layer_context_v1` (the protocol bindings exist in `bifrost-layers` with the
`client` feature). Connecting first would create the app's toplevel, so the gate refuses before connecting. With
`BIFROST_DIRECT=1` the app logs the reason once and keeps the shared-memory path.

## What is implemented

### `sdk/src/runtime.rs`

- `LAYER_CONTEXT_WIRED: bool = false`: the one place that says whether the connection can bind a layer context.
- `direct_fallback_reason(requested, vulkan, layer_manager, context_wired) -> Option<&'static str>`: the checks in
  order (switch, Vulkan, `bifrost_layer_manager_v1`, layer context); None means the direct path may run.
- `run_on` logs the fallback reason when `BIFROST_DIRECT=1`. Nothing else in the shared-memory loop changed.
- Tests `direct_gate_tests` (7): each missing piece is named, in order; the full set takes the direct path.

### `sdk/src/wl/frame.rs`

- `FrameLoop::draw_frame(sink, draw)`: unchanged behaviour; the CPU `Surface` path (one image op of a window-sized
  display list, rendered with `bifrost-2d`).
- `FrameLoop::draw_frame_canvas(sink, draw)`: new. The app records into a `DisplayList` through `Canvas`
  (`App::draw_canvas`), with no CPU copy.
- Both share `FrameLoop::present` (acquire a backing store, `Renderer::draw`, commit the submission with one
  `frame_transaction`).
- `frame_skipped(damage_empty, created, bounds_dirty)`: the skip rule, now a pure function (test
  `a_frame_without_damage_is_skipped_once_the_root_exists`).
- Damage: the whole list is rendered every frame (`Renderer::draw(..., None)`). Stores are pooled, so a damage
  rectangle would show pixels from two frames ago unless damage is accumulated per store. Left for the layer-tree views.

## Mapping to the contract

| Contract item | Where | State |
|---|---|---|
| App is a Wayland client, one poll loop over the Wayland fd, control socket and timers | `runtime.rs` `run_on` | not done (blocked on the connection's layer dispatch) |
| Frames paced by the display link from presented feedback | `frame.rs` `Pacer`, `presented` | module done; not driven yet |
| `draw_canvas` into a `DisplayList` when `draws_canvas`, else CPU surface uploaded as one image | `frame.rs` `draw_frame_canvas`, `draw_frame` | done |
| Render with `bifrost_2d::Renderer` into the next backing store, bind, commit | `frame.rs` `present`, `FrameSink` | done behind the seam; sink not implemented |
| Pixels and input never go to the shell on this path | runtime loop | not done |
| Falls back to shared memory on any failure | `direct_fallback_reason`, `run_on` | gate done; fallback is always taken today |

## Contract changes

None to public signatures. Added: `frame::frame_skipped`, `FrameLoop::draw_frame_canvas`, the private `present`,
`runtime::LAYER_CONTEXT_WIRED`, `runtime::direct_fallback_reason`.

## todo_left

1. Connection hook (blocking): `sdk/src/wl/connection.rs` needs `Dispatch<BifrostLayerManagerV1, ()>`,
   `Dispatch<BifrostLayerContextV1, ()>` for `State` (the context's `presented` and `animation_finished` events feed
   `Pacer::presented` and the animation callbacks), and a `Connection` method that binds the manager and calls
   `get_context(surface)`. Then set `LAYER_CONTEXT_WIRED` to true.
2. A `FrameSink` for the connection: `Backing` per size (`wl/backing.rs`), `acquire` takes a free slot,
   `commit` calls `bind_contents` once per slot and `commit(bifrost_layer_context commit(transaction))` with the codec
   output.
3. Runtime loop: wait on the Wayland fd (`Connection::dispatch_pending`/`flush`, `take_events`), the control socket and
   the timers in one `wait_many`; route `Event::Configure` to `ToApp::Configure` semantics; `Event::Close`; key and
   pointer events from `wl::input::Seat` to `Input`.
4. Shared-memory fallback mid-run (today it is decided at start only).
5. Damage: accumulate per backing store and pass the union to `Renderer::draw`.
6. Scroll events (`ScrollEvent`) are not in `runtime::Input` (lead decision).

## Verification

Done here:

- `cd springboard && CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-app --tests`
  passes. The warnings it prints are pre-existing and not in the changed code.
- `python3 tools/check-names.py` passes.
- Tests were written with hand-worked values. They were not run (the agent rules allow `cargo check` only). Run them on
  the lead's host: `cargo test -p bifrost-app --lib direct_gate_tests` and `cargo test -p bifrost-app --lib wl::frame`.

On Midget (Intel HD 530, Mesa ANV, Vulkan 1.4), once todo_left 1 to 3 land:

1. Set `/etc/bifrost/compositor` to `bifrost-wm` (or log in to the labwc session with `BIFROST_WM_RENDERER=vulkan` for
   the window server), then `export BIFROST_DIRECT=1` for the app's environment.
2. Start an SDK app with `bifrost pc midget open-app ID`; the app log must show no `BIFROST_DIRECT=1, shared-memory
   path` line.
3. Check the window: `bifrost pc midget state` lists the app with its own surface, and
   `bifrost pc midget tap-window APP X Y` hits a control with no shell frame involved.
4. Read the frame timings from `tools/bench.py --midget`.
