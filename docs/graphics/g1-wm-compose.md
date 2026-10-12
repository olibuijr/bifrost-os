# G1 wm-compose: scene composition in the Vulkan render server

Owner: wave G1 agent `wm-compose`. Contract: `docs/graphics-architecture.md`, "Window server",
`render_server/compose.rs`.

## What is implemented

Files: `springboard/wm/src/render_server/compose.rs`, `springboard/gpu/shaders/compose.vert`,
`springboard/gpu/shaders/compose.frag`.

- `merge_damage(rects, output)` (pure): rectangles are `[x, y, w, h]`. They are clipped to the output, empty ones
  are dropped, overlapping ones are merged repeatedly (touching ones stay apart), and more than 16 rectangles
  collapse to their bounding box.
- `SceneNode` (pure, plain data): a window, layer-shell surface, layer-tree layer or the cursor. Fields: texture
  key, source rectangle in texels, logical rectangle, affine `to_root` (a, b, c, d, tx, ty), output scale,
  opacity, corner radius, premultiplied colour, optional glass (`Glass`: blur, tint, saturation) and an optional
  `ShadowSpec` (computed by the caller with `material::window_shadow`).
- `build_items(nodes, damage, texture)` (pure): turns nodes into `DrawItem`s in output pixels, back to front. It
  drops zero-opacity nodes, nodes with nothing visible, and every item whose extent (quad, shadow reach, glass rect)
  misses the damage. A texture key the lookup cannot resolve falls back to the node's colour.
- `Push` (pure): the 128-byte push-constant block, with `to_bytes()` (std430 offsets are in the doc comment and
  tested). `content_push`, `shadow_push` and `material_push` build the three kinds of draw.
- `ComposePipelines::new(device, target_format)`: the textured/solid pipeline (`compose.vert` + `compose.frag`,
  premultiplied source-over, triangle strip, one texture and one sampler, 128 push bytes), a 1x1 white texture for
  solid draws, and a linear sampler.
- `MaterialPass` trait: `blur(cb, backdrop, region) -> Texture`. The `material` module implements it (dual-Kawase
  compute passes). `record` calls it for items with a material.
- `record(cb, pipes, target, load, damage, items, materials)`: the render pass. Per item: shadow first, then
  material (the open render pass is ended, the backdrop is blurred, a new pass is opened with `Load`), then the
  content. Each pass is scissored to the damage bounding box and stores its result.

Shaders: a unit triangle strip driven by push constants. The fragment shader does the rounded-rect clip (SDF,
half-pixel coverage), the textured or solid path, the shadow (logistic approximation of the Gaussian edge of the
shadow's rounded rect), and the glass (saturation, then the premultiplied tint over the blurred backdrop). The
result is multiplied by opacity and clip coverage.

## Mapping to the contract

| Contract item | Where |
|---|---|
| `DrawItem` (texture, quad, src, opacity, colour, clip, material, shadow) | `compose.rs`, unchanged public skeleton |
| `merge_damage` (union, clip, merge, collapse above 16) | `compose.rs` |
| "culled to damage" | `build_items` |
| "material/shadow first" | `record`, per item |
| `compose.vert` / `compose.frag` | `springboard/gpu/shaders/` |

## Public API changes (contract_changes)

- `ComposePipelines::new` takes the target format (the skeleton had none).
- `SceneNode`, `Glass`, `Push`, `MaterialPass`, `build_items`, `record`, `content_push`, `shadow_push`,
  `material_push`, `item_extent`, `shadow_extent` are new public items in `compose.rs`.
- `build_items` takes the texture lookup as `&dyn Fn(u64) -> Option<Texture>`, so the builder is testable without
  a GPU.
- `src` in `DrawItem` is in texels, and a width or height of 0 means the whole texture. `record` normalises it.

## Assumptions to verify with the other owners

- The descriptor layout: `compose.frag` declares a `texture2D` at set 0 binding 0 and a `sampler` at binding 1.
  `gpu-pipeline` must create separate texture and sampler bindings with those numbers (`set_texture(0, ..)`,
  `set_sampler(1, ..)`).
- The push-constant range covers the vertex and fragment stages (the block is read in both).
- `Device::new_render_pipeline` must accept `strip: true` with no vertex layout (the corners come from push
  constants).
- `CommandBuffer::render_encoder` must allow `render_area` and `set_scissor` on the same region.
- `CommandBuffer` must not require an open encoder for `MaterialPass::blur` (the compose code ends every pass
  before calling it).
- The transform convention of `SceneNode::to_root` is x' = a x + c y + tx, y' = b x + d y + ty (the reference
  platform's convention). `layers::Affine` must use the same one when the layer tree is turned into nodes.

## Not done (todo_left)

- Layer-tree nodes: nothing converts `layers::PresentedLayer` into `SceneNode` yet. That needs the
  contents-to-texture map from `layers::Context` and is left to the window glue (`wm-layers`/`wm-compose` with the
  dispatch code).
- Per-rectangle scissoring: `record` scissors to the damage bounding box, not each rectangle. Correct, but it
  redraws the area between separate damage rectangles.
- Clip with a transform: the clip is the transformed quad's bounding box, so a rotated rounded window is clipped
  as a rectangle. Exact rotated clips need a per-item rounded mask.
- Shadow colour and the shadow approximation are not compared against the reference look yet (Midget).
- Nothing has run: this crate only compiles here. The GPU path needs `gpu-tests` on Midget (below).

## Verification

Host, with the permitted check only (no build or test run on this worktree's target):

```
cd springboard
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-wm --tests
python3 tools/check-names.py
```

Pure tests in `compose.rs` (`cargo test -p bifrost-wm render_server::compose`, to run once the crate is built): damage
merging (empty, outside, clipped, overlap, touching, chained, 16 and 17 rectangles), push-block offsets and bytes,
scaled quads, culling, ordering, transforms, rounded clips, shadow and glass extents, and the three push builders.
The expected values were worked out by hand.

On Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): once `gpu-pipeline` and `gpu-command` land, render one window
quad with a rounded clip and a shadow, plus one glass region, through `record` into a `bifrost-gpu` texture, read it
back with `Texture::read_region`, and compare it with the CPU raster of the same scene. Then time `record` on a
typical desktop (`tools/bench.py --midget`, budget under 1 ms CPU).
