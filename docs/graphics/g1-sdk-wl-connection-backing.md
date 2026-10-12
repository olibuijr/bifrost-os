# G1: SDK Wayland connection and backing stores (`sdk-wl-connection-backing`)

Part of the graphics core rebuild (docs/graphics-architecture.md, "SDK"). Owner files:
`springboard/sdk/src/wl/connection.rs` and `springboard/sdk/src/wl/backing.rs`.

## What is implemented

### connection.rs

- `Connection::connect(app_id, title)`: connects to `WAYLAND_DISPLAY` (`wayland-client` 0.31), reads the registry
  with `registry_queue_init`, binds `wl_compositor` (4..=6), `xdg_wm_base` (1..=6), and optionally
  `zwp_linux_dmabuf_v1` (4..=5), `wp_presentation` (1..=2) and `wl_seat` (5..=9). It makes the surface, the xdg
  surface and one toplevel with app id and title, commits, and roundtrips twice so the first configure and the
  dmabuf feedback have arrived before it returns.
- xdg protocol duties: `xdg_wm_base.ping` is answered with `pong`; `xdg_surface.configure` is acknowledged at once.
- Events for the frame module, drained with `take_events()`: `Event::Configure { width, height }`, `Event::Close`,
  `Event::BufferReleased { tag }`.
- linux-dmabuf v4 default feedback: `format_table` (read from its fd), `main_device`, `tranche_formats`,
  `tranche_done`, `done`. `dmabuf_formats()` returns the (fourcc, modifier) pairs in preference order and
  `dmabuf_main_device()` the dev_t.
- `create_dmabuf_buffer(&DmaBuf, tag)`: `zwp_linux_dmabuf_v1.create_params`, one `add` per plane (modifier split into
  hi and lo), then `create_immed` with the tag as the `wl_buffer` user data. The release event maps back to the tag.
- Pure, tested helpers: `parse_format_table` (16-byte entries), `parse_dev_t`, `tranche_formats` (u16 indices),
  `Feedback` (accumulates the events).
- Layer manager: `Globals::layer_manager` records the registry name and version of `bifrost_layer_manager_v1`
  (`LAYER_MANAGER_INTERFACE`). It is not bound here because the protocol bindings belong to `layers-protocol`.

### backing.rs

- `Pool`: pure slot bookkeeping with states `Free`, `Acquired`, `Committed`. Transitions: `acquire`, `commit`,
  `cancel`, `release`. Wrong transitions and out-of-range slots return `PoolError`.
- `encode_tag` / `decode_tag`: store id in the high 32 bits, slot in the low 32 bits.
- `modifiers_for(formats, fourcc)`: the modifiers the feedback offers for one format, deduplicated, in order.
- `Backing::new(conn, device, store, w, h, format, formats, count)`: makes `count` (`POOL_SIZE` = 3) exportable
  textures through `bifrost_gpu::Device::new_texture` with the feedback's modifiers, exports each as a dma-buf, and
  wraps it as a `wl_buffer` tagged with (store, slot).
- `acquire`, `texture(slot)`, `buffer(slot)`, `commit(slot)`, `cancel(slot)`, `release_tag(tag)` (returns whether the
  tag was this store's), `tag(slot)`, `free_slots()`. `Drop` destroys the `wl_buffer`s.

## How it maps to the contract

- The SDK is a Wayland client of the window server: xdg toplevel, dma-buf backing stores drawn with `bifrost-gpu`,
  and events on the connection. Input is left to `input`.
- Backing stores are per layer. The layer owner calls `buffer(slot)` and `bind_contents`, then `commit`, and feeds
  `Event::BufferReleased` into `release_tag`.
- Explicit sync (syncobj, sync_file) is not done here; it belongs to the frame module and `bifrost-gpu::sync`.

## Deviations from the skeleton

The skeleton's `connection.rs` and `backing.rs` contained only a doc line and no `todo!()` bodies or public API, so the
API above is new. `wp_cursor_shape_manager_v1` is not bound: the Rust binding sits behind wayland-protocols'
`unstable` feature, which `bifrost-app` does not enable and Cargo.toml is not mine to change.

## What is left (todo_left)

- `input` binds the seat's dispatch (`Dispatch<WlSeat, ()>` is an ignore stub here) and cursor-shape, which needs the
  `unstable` feature on `wayland-protocols` in `sdk/Cargo.toml`.
- `frame` owns `wp_presentation` feedback; the global is bound but no feedback objects are made.
- `layers-protocol` must bind `bifrost_layer_manager_v1` from `Globals::layer_manager` and provide `get_context` and
  `bind_contents` (they can use `Connection::surface()`).
- Multi-store routing: the caller keeps one `Backing` per layer and forwards each release tag to all of them.
- Registry global removal and reconnect are not handled.
- Nothing has run against a compositor yet: `bifrost-gpu`'s `Device::new_texture`, `export_dmabuf`, and
  `Device::open` are still `todo!()` in the gpu crate.

## Verification

- `CARGO_TARGET_DIR=springboard/target-agents cargo check --offline -j 2 -p bifrost-app --tests` passes, with no
  warnings from `sdk/src/wl/`.
- `python3 tools/check-names.py` passes.
- Unit tests cover the format table parser, dev_t parser, tranche index resolution, feedback accumulation, pool
  transitions, tag encoding and modifier selection. Expected values were worked out by hand. They were not executed
  in this round because the agent build rules forbid `cargo test`.

## On Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Build the fast profile and run an SDK test app with `BIFROST_DIRECT=1` once `bifrost-gpu` is implemented.
2. Confirm the feedback: the format table should list `XRGB8888`/`ARGB8888` with the linear and Y-tiled modifiers
   that ANV advertises, and `dmabuf_main_device()` should match the render node.
3. Check the first configure arrives before the first frame and that `Close` is delivered when the window is closed.
4. Check that `BufferReleased` events arrive after the compositor is done with a committed slot. No slot should stay
   Committed past one frame in the steady state.
