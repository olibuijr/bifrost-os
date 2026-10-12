# G2 `wm-scene`: building the window stack as scene nodes

Owner: wave G2 agent `wm-scene`. Branch `g2/wm-scene`. Contract: `docs/graphics-architecture.md` (Window server),
work table: `docs/graphics-waves.md` (G2 row 3).

## What is implemented

`springboard/wm/src/render_server/scene_build.rs` (new, declared in `render_server/mod.rs`):

- `build(state, output, t, server) -> Vec<SceneNode>`, back to front:
  1. background and bottom layer-shell surfaces (`layer_map_for_output`, bottom first);
  2. windows in stacking order (`state.space.elements()`): a shadow node per window carrying
     `material::window_shadow` (active or inactive, from the toplevel's `Activated` state), the window's
     `ext-background-effect` regions as glass nodes (`GLASS_BLUR`, `GLASS_TINT`, `SATURATION`), then the surface tree
     (toplevel and subsurfaces, placed by the surface view offsets, clipped to `WINDOW_RADIUS` = 17 px);
  3. `bifrost_layer_v1` contexts in id order, each from `ContextStore::presentation(ctx, t)`, one `layer_nodes` per
     presented layer;
  4. top and overlay layer-shell surfaces.
  A held session lock draws only the lock surface.
- Buffers: a client dma-buf is imported once per `wl_buffer` (key = hash of the buffer's object id) through
  `Importer::import_dmabuf` (zero copy); the node names the key and `compose::build_items` looks the texture up.
- `layer_nodes(presented, bound, scale)` (pure): shadow carrier, glass material, border (outer rounded rect plus an inset
  background), background colour, bound contents as a textured node. Colours are premultiplied; the layer's
  `opacity` (accumulated by the tree) goes on each node's `opacity`. Rounded clip only for a layer that
  `masks_to_bounds`.
- `node_bounds`, `device_rect` (pure): device bounds of a node including its shadow reach.
- `diff_damage(prev, cur)` (pure): nodes that differ at the same position contribute old and new bounds; an added or
  removed node contributes its bounds.
- `DamageHistory::frame_damage(age, changed, output)` (pure): buffer-age damage. Age 0 or an age beyond the history
  redraws the whole output; otherwise it unions this frame's changes with the changes of the last `age - 1` frames
  (merged with `compose::merge_damage`).

`springboard/wm/src/scene.rs` (the GLES scene) is untouched and still builds the GLES path.

## How it maps to the contract

- `importer` and `layers` come from `Server` (`server.rs`), so `build` takes `&mut Server`; the skeleton's
  `build(state, output, t)` had no place for them. This is the one signature change (see `contract_changes`).
- The `SceneNode` type and the glass parameters are the ones in `compose.rs`; no shader or push layout changed.
- Layer shadows: the shadow sigma is half the layer shadow radius (`sigma = radius * 0.5`, times the scale), the offset
  is the layer's offset, the colour alpha is `shadow.color.a * shadow.opacity`.
- Material on a layer: `Material::Glass` maps to `compose::Glass`; the tint is premultiplied.

## Tests

`#[cfg(test)] mod tests` in `scene_build.rs`: 16 host tests with values worked out by hand (layer background,
border inset with radius 10 -> 8, shadow sigma 10 with premultiplied colour, bound and unbound contents, glass tint
0.25 -> 0.25 premultiplied, opacity, node bounds with transform and with a 3-sigma shadow reach, damage diff for a
moved, added and removed node, buffer age 0, 1, 2, beyond history, and capacity).

This branch was checked with `cargo check -p bifrost-wm --tests` only (passes). The tests have not been run; the
lead's test pass runs `cargo test -p bifrost-wm scene_build`.

## What is left (todo_left)

1. Nothing calls `scene_build::build` yet. The DRM and nested backends (`wm-drm-vulkan`, `wm-nested-headless`) must
   call it from their frame loop, pass the result to `Server::frame`, and feed `diff_damage` plus `DamageHistory`
   with the target's buffer age.
2. shm buffers produce no node. Reading shm bytes safely needs a helper in `springboard/gpu` (unsafe is kept there);
   then `Importer::upload_shm` with the surface's damage (`RendererSurfaceState::damage`).
3. The cursor and the boot fade are not in the stack; the caller appends them.
4. Ancestor masks: a layer inside a `masks_to_bounds` ancestor is not clipped by it (`PresentedLayer::clip` is not
   used). Needs a clip rectangle on `SceneNode` (a contract change).
5. `ContentsGravity` other than `Resize` is not mapped.
6. Layer contexts are placed in output logical space by their tree; the mapping of a context to its window (for
   stacking the layer tree among windows) lives in the layer glue and is not yet used.
7. `DamageHistory` is per output and must be reset when the output's targets are reallocated (`reset`).
8. Dma-buf keys are never forgotten: `Importer::forget` must run when a `wl_buffer` is destroyed (buffer destruction
   is not hooked yet).
9. The window radius is 17 px from CLAUDE.md; `render/src/ui.rs` still says 10 pt (`WINDOW_RADIUS_PT`). Resolve in
   the design pass.
10. Glass constants (`GLASS_BLUR` 20, `GLASS_TINT` 25 % white) are first guesses; tune against the shell's glass
    on Midget.

## How to verify on Midget

Midget: Intel HD 530, Mesa ANV, Vulkan 1.4. The session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`.

This branch alone cannot be run on Midget: item 1 above is not done. Once a backend calls `build`:

1. Build the fast profile (`cargo build --offline --profile fast`, in `springboard/`) and push the shell to Midget as in `docs/bifrost-pc-workflow.md`.
2. Publish and update Midget as in `docs/bifrost-pc-workflow.md`, then set the renderer and compositor:
   `echo vulkan | sudo tee /etc/bifrost/renderer` and `echo bifrost-wm | sudo tee /etc/bifrost/compositor`, then
   reboot with `bifrost pc midget update --restart`.
3. Check the recovery state with `python3 tools/midget.py recovery` (`phase=accepted`).
4. Take a screenshot with `bifrost pc midget screen PATH` and look at: the menu bar and wallpaper (layer
   surfaces), an xdg window with its shadow (active window darker than inactive), glass under a panel that asks for
   blur, and the dock.
5. Move a window and check on the stats that only its old and new places are redrawn (`bifrost-wm-frames.jsonl`,
   `damaged_px` well below the output size).
6. Revert with `echo labwc | sudo tee /etc/bifrost/compositor` if anything is wrong.
