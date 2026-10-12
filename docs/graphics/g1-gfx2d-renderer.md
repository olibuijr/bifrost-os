# G1 gfx2d-renderer: the display list executor

Owner: wave G1 agent `gfx2d-renderer`. Contract: `docs/graphics-architecture.md`, "bifrost-2d". Code:
`springboard/gfx2d/src/renderer.rs`.

## What is implemented

- `Renderer::new(device)`: builds the shape pipeline (`Shapes::new`, target `Bgra8Unorm`), the glyph atlas
  (`GlyphAtlas::new`), the image cache (`ImageCache::new`, 64 MiB budget) and a ring of three MAPPED instance buffers
  (`VERTEX | MAPPED`, 256 instances each, grown on demand).
- `Renderer::draw(cb, list, target, damage)`:
  - walks `DisplayList::ops` in order with three stacks: transform (`mul` composes `a(b(p))`), device-space clip
    (bounding box of the pushed rectangle, intersected), and opacity (multiplied, clamped to 0..1);
  - culls drawing ops by their recorded device `bounds` when `damage` is set;
  - encodes shape ops as `shapes::Instance`s (`encode`): `FillRect`, `FillRRect`, `StrokeRRect`, `Shadow` (shape moved
    by the offset, sigma = blur / 2), `Material` (its tint). The colour is unpremultiplied to straight RGBA, with the
    opacity stack multiplied into alpha;
  - batches instances by key (`batch_runs`, a pure function over `BatchKey::{Shape, Mask(tex), Image(tex)}`): equal
    neighbours share one draw, order is kept;
  - records one render pass: `LoadAction::Load` with `render_area` = the damage rounded outwards and clamped to the
    target, or `LoadAction::Clear` with no area when `damage` is `None`; sets the shape pipeline (cached per target
    format), pushes the target size as push constants (two little-endian f32), binds the ring buffer, draws
    `draw(4, count, 0, first)` per run (triangle strip, per instance);
  - fills `FrameStats` (ops, draws, instances, encode_us; glyphs and images uploaded stay 0).
- Pure, host-tested helpers (no GPU): `mul`, `bbox`, `intersect`, `damage_region`, `straight_color`, `batch_runs`,
  `Walk` (the stacks), `encode`, `instance_bytes`. The tests are in `renderer.rs` (`mod tests`), with values worked out
  by hand.

## Mapping to the contract

| Contract item | Where |
|---|---|
| Clip / transform / opacity stacks | `Walk`; the clip is a device-space rectangle |
| Instances with the transform per instance | `Instance::xform` / `xform_t` hold the current transform; `rect` stays in local pixels |
| Batching by pipeline and texture, under 50 draws | `batch_runs`; shape ops are one run, so a frame of shapes is one draw |
| Damage scissor, Load vs Clear | `draw`: `render_area` and `LoadAction` |
| `FrameStats` | `draw` |

## Not done (todo_left)

1. **Gradients** (`Op::LinearGradient`): `Instance` has no field for the gradient end point, and only two colours
   fit (`color`, `color2`). The executor skips gradients. Needed: the end point (two floats) in the instance layout
   (a contract change for `gfx2d-shapes`), and the stops beyond the first and last.
2. **Images and tinted images** (`Op::Image`, `Op::ImageTinted`): need the image cache's upload and lookup
   (`ImageCache` has only `new` in the skeleton) and an image pipeline. `BatchKey::Image` is in place; the draw loop
   ignores image runs until they are encoded.
3. **Text** (`Op::Text`): needs the glyph atlas lookup and the layout of a `TextRun` into `Mask` quads
   (`GlyphAtlas` has only `new`). `BatchKey::Mask` is in place; `glyphs_uploaded` stays 0.
4. **Paths** (`Op::Path`): `paths::tessellate` gives triangles, but no pipeline draws triangle lists yet (the shape
   pipeline draws instanced quads). Needed: a triangle-list pipeline in `shapes` (or a `paths` pipeline).
5. **Rounded clips**: `Instance::clip` is a rectangle. A rounded `PushClip` is applied as its bounding rectangle. An
   SDF clip needs the radius in the instance, or a clip mask.
6. **Group opacity**: `PushOpacity` multiplies into each op's alpha. Overlapping children of a group are then
   composited op by op, not as a group. Correct for non-overlapping content; a layer needs an offscreen group.
7. **Material**: drawn as a tint fill. The blur belongs to the window server (`render_server/material.rs`).
8. **Instance buffer lifetime**: the ring reuses a buffer three frames later. The caller must have waited for the
   submission of that frame. The command buffer does not keep the buffer alive (its `keep` list is private to
   `bifrost-gpu`); a `bifrost-gpu` `CommandBuffer::keep_buffer` would make it safe.
9. **Shadow quad**: the instance `rect` is the shape rectangle (offset applied). The shadow shader must expand the
   quad by about 3 sigma itself, because the instance has no room for the outer bounds. Owner: `gfx2d-shapes`.
10. **Push constants**: the target size (width, height as f32) is pushed as 8 bytes. `gfx2d-shapes` must declare
    `push_constant_bytes >= 8` and read them in `shape.vert`.
11. No GPU tests: `bifrost-2d` has no `gpu-tests` feature (that would need a `Cargo.toml` edit). The GPU path is
    checked on Midget (below).

## Verify

Host (no GPU needed):

```
cd springboard
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-2d --tests
```

Then, after the modules merge, `cargo test -p bifrost-2d` runs the pure tests (`mul`, `bbox`, `intersect`, damage,
colour, batching, stacks, encoding, instance bytes).

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): once `gfx2d-shapes` and `gpu` are merged, a frame of shape ops drawn into
a `Bgra8Unorm` texture with `read_region` and compared against the CPU `Surface` for the same display list, with
the pixel tolerance of the shapes module. Then `bifrost pc midget` screenshot of a window drawn this way. Budget:
under 50 draws per window and encode time in `FrameStats::encode_us` under 1 ms for a typical window.
