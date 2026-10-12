# G1 gfx2d-images: image cache and path tessellation

Wave G1 part `gfx2d-images` of the graphics core (`docs/graphics-architecture.md`, "bifrost-2d"). Owns
`springboard/gfx2d/src/images.rs` and `springboard/gfx2d/src/paths.rs`. Nothing else in the crate changed.

## What is implemented

### Image cache (`images.rs`)

- `ImageCache::new(device, budget_bytes)` keeps a `Device` clone and a byte budget (`DEFAULT_BUDGET_BYTES` = 64 MiB
  is exported for the renderer). A zero budget is refused.
- `ImageCache::get(&ImageRef)`: returns the texture for `ImageRef::id`. On a miss it creates an `Rgba8Unorm` texture
  (sampled and copy-destination, not exportable) and uploads the premultiplied pixels with `replace_region`. A resident
  id with a different size is replaced. A hit is marked most recently used.
- `ImageCache::update(&ImageRef, Region)`: re-uploads one region of an image that keeps its id (for example a
  progressively decoded image). Falls back to a full `get` when the image is not resident.
- `remove`, `contains`, `used_bytes`, `len`, `is_empty`, `budget`.
- Eviction: `LruBook` is the pure bookkeeping (keys, bytes, recency tick, `BTreeMap` from tick to key). Inserting past
  the budget evicts least recently used entries, oldest first. The entry just inserted is never evicted, so an image
  bigger than the whole budget stays alone. Evicted entries drop their `Texture` handle; the GPU memory is freed when
  the last handle goes (the `bifrost-gpu` submission keeps its own references until the GPU is done).
- Pixel packing: `Image::px` is one `u32` per pixel with R in the low byte, so `to_le_bytes` produces R, G, B, A in
  memory, which is the `Rgba8Unorm` layout.

### Path tessellation (`paths.rs`)

- `tessellate(&PathData, PathPaint) -> Vec<PathVertex>`: a triangle list in device pixels. Vertices carry coverage 1
  on the outline and interior, and 0 one pixel outside it (the 1 px fringe).
- `flatten`: lines, quads and cubics. Curve segments follow the chord bound: quad `n = ceil(sqrt(|p0-2c+p1| / (4 tol)))`,
  cubic `n = ceil(sqrt(3 M / (4 tol)))` with `M = max(|A|, |B|)`, `tol = 0.25` px, capped at 512 segments.
  `Close` closes a subpath. Subpaths with non-finite points are dropped.
- Fills (`FillNonZero`, `FillEvenOdd`): every edge endpoint and every pairwise crossing is a horizontal breakpoint, so
  within a band no two edges cross. Each band walks its edges left to right with the winding count (non-zero: count
  != 0; even-odd: count odd) and emits the inside spans as trapezoids. Edges where the inside status flips get a 1 px
  fringe along the outward normal. Horizontal boundaries get the same fringe, piece by piece between span endpoints.
- Strokes (`Stroke(w)`): the union of a rectangle per segment, a round join where the turn is visible
  (`hw * theta^2 >= 0.25`), and round caps (or a dot for a zero-length subpath), every outline oriented
  counter-clockwise and filled non-zero, so overlaps are the union. Circles have at least 24 sides.
- The output is unoriented: the consumer must not cull these triangles. Fringe triangles have a 0 vertex, so the
  interior of a shape is the sum of the all-coverage-1 triangles.

## How it maps to the contract

| Contract item (`docs/graphics-architecture.md`) | Where |
|---|---|
| "images: texture cache keyed by image id with LRU eviction" | `ImageCache`, `LruBook` |
| "paths tessellated on the CPU into triangles (fill: even-odd/non-zero; stroke: expanded polygons)" | `tessellate` |
| Public signatures of the skeleton | unchanged: `ImageCache::new`, `tessellate`, `PathVertex` |
| `unsafe` only in `springboard/gpu` | none added |

No contract changes.

## Verification done here

- `cargo check --offline -p bifrost-2d --tests` passes in this worktree (no warnings from `images.rs` or `paths.rs`).
- `python3 tools/check-names.py` passes.
- The pure logic was run as a scratch harness (`rustc --test` on copies of the two modules with stubs for the GPU and
  canvas types, outside the repo): 31 tests pass. They cover: LRU order, touch, reinsert, oversized entries, byte
  packing, region checks, flatten counts against the chord bound, square area 100 with a fringe of 40, overlapping
  squares (union 175, XOR 150), holes under both rules, reversed holes, the pentagram difference (inner pentagon),
  a circle from four cubics (within 1%), a round-capped segment (40 + one 24-gon disc), an L-shaped round join,
  a closed ring (76 + pi), a dot, zero-area paths, and the slanted fringe lying exactly 1 px out.
- These tests are in the `#[cfg(test)] mod tests` of each file and run with `cargo test -p bifrost-2d` (not run
  here, as the workspace rules forbid it in this round).

## What is left (todo_left)

- GPU tests: `ImageCache` needs a real `Device`; the upload, eviction-frees-texture and `update` paths are not yet
  covered by a `gpu-tests` feature test.
- Renderer integration: `Renderer` does not call `ImageCache::get` or `tessellate` yet (the `gfx2d-renderer` part).
- Performance: the crossing search is O(n^2) with a y-sorted prune, and the band sweep is O(breakpoints x active
  edges). Fine for UI paths (hundreds of edges). Whole-page vector art, or a stroke of a long curve with many joins,
  needs a bench before it goes into a frame. Add `bench_gfx2d_paths` to `tools/bench.py` with the Midget run.
- Curve joins in strokes are round everywhere; miter and bevel joins (the `Stroke` paint has no join style) are not
  in the `PathPaint` contract and are not implemented.
- Fringe overlap: at convex corners the fringe strips leave a small notch rather than overlapping, and at reflex
  corners two strips can overlap and add alpha. Acceptable for coverage 1 px, but a corner-exact fringe would need a
  miter pass.
- Even-odd and non-zero are tested on closed shapes only; open subpaths in fills are closed implicitly, as the
  reference model does.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Host: `cargo test -p bifrost-2d` in `springboard/` (the pure tests above), then `python3 tools/check-names.py`.
2. Device: once the renderer draws paths (`gfx2d-renderer`), draw the same display list with `bifrost_render::Surface`
   (CPU) and with `bifrost-2d` on Midget with `BIFROST_DIRECT=1`, then compare the edges at 1.5625 scale: fringe
   width 1 px, no gaps between bands, no dropped joins.
3. Images: open an app with photographs (`apps/browser` or Photos) and check that scrolling a large image does not
   re-upload (the bench counter `images_uploaded` in `FrameStats` stays at 0 on a steady frame).
4. Record the path and image timings with `tools/bench.py --midget` (budgets: window frame under 4 ms GPU, under
   1 ms CPU) and append them to `docs/perf-history.jsonl`.
