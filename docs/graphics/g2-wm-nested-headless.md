# G2 wave: nested and headless Vulkan paths (`wm-nested-headless`)

Owner files: `springboard/wm/src/nested.rs`, `springboard/wm/src/headless.rs`, `springboard/wm/tests/vulkan_headless.rs`.
Contract: `docs/graphics-architecture.md`; wave row: `docs/graphics-waves.md` (G2 table, row 2).

## What is implemented

### Headless (`headless.rs`)

- `VulkanHeadless::new(w, h)`: a `render_server::server::Server` on the default device (`Device::open(None)`) with a
  `Bgra8Unorm` target, and an offscreen render target with `RENDER_TARGET | SAMPLED | COPY_SRC`.
- `render(nodes, damage)`: composites through `Server::frame` with `LoadAction::Clear` to the desktop clear colour,
  waits for the submission, and reads the whole target back as BGRA rows, top first.
- `resize(w, h)`: reallocates the target when the size changes.
- Pure helpers: `bgra_to_rgba`, `rgba_at`, `full_damage`. The GLES `Headless` type is unchanged.
- The shared Wayland client dispatcher flushes replies immediately after dispatch, independently of rendering. This
  keeps initial registry and sync roundtrips responsive even if first-frame GPU setup takes time.

### Nested (`nested.rs`)

- When `render_server::selected()` is true (`BIFROST_WM_RENDERER=vulkan` or `/etc/bifrost/renderer`), `init` builds a
  `VulkanHeadless` at the window size. If that fails it logs the error and keeps the GLES path.
- The winit window stays GLES. Each redraw, the Vulkan frame is read back and imported with
  `GlesRenderer::import_memory` (`Fourcc::Argb8888`), then drawn as one `TextureRenderElement` through the same
  damage tracker. This is the shm fallback: one readback copy per frame. A Vulkan swapchain would remove the copy and
  is not built.
- The scene nodes passed to the Vulkan frame are an empty list for now, so the nested Vulkan window shows only the
  clear colour. Building `SceneNode`s from the window stack is `wm-scene`'s job.

### End-to-end test (`tests/vulkan_headless.rs`, feature `gpu-tests`)

- `nodes_land_at_their_rectangles`: a blue background and a red 16x16 node at (8, 8) on a 64x48 target. The pixel at
  (16, 16) is red, (7, 7) and (24, 24) are background.
- `damage_limits_what_is_drawn`: damage of 8x8 at the origin; the blue node is drawn there and the rest is clear.
- `an_empty_scene_is_the_clear_colour`, `resize_reallocates_the_target`, `a_frame_without_damage_has_no_pixels_drawn`.
- The expected values are worked out by hand. The clear colour (0.06, 0.06, 0.08) is about (15, 15, 20) in 8 bits;
  the asserts allow a rounding of one step.

## Mapping to the contract

| Contract item | Where |
|---|---|
| Vulkan render server on the nested backend, copy or blit to the window | `nested.rs`, `VulkanPresenter::element`, `vulkan_presenter` |
| Swapchain-free shm fallback for nested | `nested.rs`, readback plus `import_memory` |
| Headless renders to a texture and reads back | `headless.rs`, `VulkanHeadless::{new, render, resize}` |
| End-to-end harness behind `gpu-tests` | `tests/vulkan_headless.rs` (placement, damage, resize) |
| Falls back when Vulkan is missing | `nested.rs`, `vulkan_presenter` logs and returns `None` |

## Tests run

- `CARGO_TARGET_DIR=.../springboard/target-agents cargo check --offline -j 2 -p bifrost-wm --tests`: passes.
  The two warnings are in the `bifrost-gpu` crate, not in this change.
- `python3 tools/check-names.py`: no hits.
- Not run: the `gpu-tests` tests (no Vulkan device in the agent sandbox), and the nested window (no display).
  The host pure tests in `headless::tests` (6) were not executed either: this wave runs `cargo check` only.

## What is left (todo_left)

- Scene nodes: `wm-scene` must build `Vec<SceneNode>` from the window stack for both the nested and headless paths.
  Until then the Vulkan frames are the clear colour only.
- Wayland end-to-end harness (`tests/vulkan_wayland.rs`): start a headless `bifrost-wm` in-process, connect a
  wayland-client test client, map an shm toplevel, a layer-shell surface and a `bifrost_layer_v1` context with an
  animated layer, step frames and assert placement, damage and that the animation advances without client commits.
  It needs the window-stack scene building above and the layers dispatch wired to the server.
- Blur and glass in the nested path: `material` is used by `Server::frame`, but the scene nodes that carry glass are
  not built yet.
- The output transform: the winit output is `Transform::Flipped180`. The Vulkan readback is top-first and is imported
  with `flipped = false`, so the orientation must be checked on a real window; the GLES path may need the same flip.
- A Vulkan swapchain (no readback) for the nested window.
- `nested.rs` has no pure logic of its own, so it has no unit tests; the helpers it uses are tested in `headless.rs`.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Host: `cd springboard && cargo check --offline -p bifrost-wm --tests` (the build check in this wave).
2. On Midget or Titan with a Vulkan device, run the GPU tests:
   `cargo test --offline -p bifrost-wm --features gpu-tests --test vulkan_headless` (one run per batch, not in this wave).
3. Nested window on a desktop: `BIFROST_WM_RENDERER=vulkan cargo run --offline -p bifrost-wm` in a session with a
   Wayland or X11 display; the log should not show "vulkan render server unavailable". With no scene nodes the window
   is the clear colour, which confirms the path runs.
4. Midget: the session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says `bifrost-wm`; `/etc/bifrost/renderer` set to
   `vulkan` selects this path. Check `journalctl` for the fallback message.
