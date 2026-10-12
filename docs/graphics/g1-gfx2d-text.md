# G1 gfx2d-text: glyph atlas and text layout

Owner: wave G1 agent `gfx2d-text`. Contract: `docs/graphics-architecture.md` ("bifrost-2d", Text).
Code: `springboard/gfx2d/src/text.rs` (the only file changed).

## What is implemented

- **Shelf packer** (`ShelfPacker`, pure): rectangles go left to right on shelves; a rectangle takes the shortest
  existing shelf that is tall enough and has room, else a new shelf opens below. `clear()` resets it.
- **GlyphKey** (`face`, `glyph`, `size_64`, `subpixel` in quarter pixels, `vars`): `GlyphKey::new` fills it;
  `vars_hash` is FNV-1a over tags and value bits (deterministic). A glyph id of 0 (emoji, or a character the face
  lacks) folds the code point into `vars`, so two such characters do not share an entry.
- **Rasterising** (`rasterize`): `Font::render_var` on one character with the run's size and variation values,
  so the bitmap is the CPU image pixel for pixel at offset 0 (test `unshifted_bitmap_matches_the_cpu_image`).
  A quarter-pixel offset is applied to the 1x bitmap by linear interpolation (`shift_alpha`), one column wider.
- **Layout** (`placements`, pure): origin (`run.x`, `run.y`) rounded to whole pixels; the pen starts at `PAD` (2)
  inside the image and advances by `Font::measure_var` per character. The pen's fraction becomes the quarter
  offset (`split_pen`, with carry). No kerning, matching the CPU path.
- **Atlas** (`GlyphAtlas`): one `R8Unorm` 2048x2048 texture (`SAMPLED | COPY_DST`). `layout()` rasterises and
  uploads missing glyphs with `Texture::replace_region`, then appends one `shapes::Instance` per glyph. When the
  atlas is full it is reset and the run is laid out again from the start; a run that does not fit an empty atlas
  is an error (`Error::Invalid`).
- Colours: the packed premultiplied `u32` is converted to straight RGBA 0..1 for the instance.

## Contract mapping

| Contract item | Where |
|---|---|
| `GlyphAtlas::new(device)` (R8 atlas, shelf packing, page reset) | `GlyphAtlas::new`, `ShelfPacker`, `reset` |
| `GlyphKey` hashing | `GlyphKey`, `glyph_key`, `vars_hash` |
| Rasterise at the subpixel offset via `render/src/font.rs` | `rasterize` |
| Upload with `Texture::replace_region` | `GlyphAtlas::insert` |
| `TextRun` to `shapes::Kind::Mask` instances matching `Font::measure` | `placements`, `mask_instance`, `GlyphAtlas::layout` |

Instance encoding for Mask (the shader owner must read it the same way): `rect` = quad x, y, w, h in device px;
`params[2]` = `Kind::Mask` as a number; `color` = straight RGBA 0..1; `color2` = atlas x, y, w, h in texels
(divide by `text::ATLAS_SIZE`); `xform` = identity; `clip` = none (the renderer applies its transform and clip
stacks, as for other shapes). The atlas texture is `GlyphAtlas::texture()`.

## Contract changes

None to the skeleton's public signatures. Added public items in `text.rs` only: `ATLAS_SIZE`, `PAD`, `SUBPIXELS`,
`GlyphKey::new`, `vars_hash`, `glyph_key`, `ShelfPacker`, `split_pen`, `shift_alpha`, `Bitmap`, `rasterize`,
`Placement`, `placements`, `mask_instance`, `GlyphAtlas::{texture, entry, len, is_empty, reset, layout}`.

## Tests

Pure tests in `text.rs` (no GPU): packer placement, best-fit, overflow and clear; `split_pen`; `shift_alpha` by
hand; GlyphKey and vars hashing; instance encoding and colour; layout pens against `Font::measure` for
"Hello, World!" at 16 px using `springboard/assets/SFUI.ttf`; the unshifted bitmap against `Font::render`.
Verified with `cargo check --offline -p bifrost-2d --tests` (no test run in this round, per the build rules).
GPU upload tests are not written yet (see todo_left).

## Known gaps (todo_left)

1. **True subpixel rasterisation.** The quarter offset is a 1x linear shift, which softens stems slightly. The fix
   belongs in `render/src/font.rs`: a `Font::render_glyph_at(gid, style, vars, dx_quarter)` that calls
   `glyph_mask` with the pen offset (the CPU path already has that code in `compose`).
2. **Kerning.** Not applied, because `Font::render` applies none and `Font` does not expose its kern table
   (`render/src/kern.rs` parses it but nothing calls it). When the CPU path gains kerning, `placements` must read
   the same values.
3. **Cascade fallback.** `placements` uses one face. `font::cascade_runs` splits text across faces; the renderer
   should call `layout` per cascade run with each face's `run.face`.
4. **Emoji.** A single emoji character is rasterised through `render_var` (the colour bitmap), so it works, but
   mixed-cluster emoji (ZWJ sequences) are laid out per character, not per cluster.
5. **Upload batching.** Each new glyph is one `replace_region` (one staging copy). Batch the new glyphs of a frame
   into one copy command once `CommandBuffer::copy_buffer_to_texture` is implemented.
6. **Atlas reset policy.** Full reset, not per-glyph LRU. Fine for now; LRU needs stamps on entries.
7. **GPU tests.** The atlas upload path needs a device; add `#[cfg(feature = "gpu-tests")]` tests once
   `bifrost-2d` declares that feature (it does not yet).

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Host: `cd springboard && cargo test --offline -p bifrost-2d text::` (pure tests; needs the font asset).
2. Once `renderer.rs` draws a `TextRun` through `GlyphAtlas::layout`: `bifrost pc smoke` on a label, then on
   Midget with `tools/midget-push-shell.sh` and a screenshot. Check the glyphs against the CPU image of the same
   string (the glyph positions must not drift across a line; compare the end x with `Font::measure`).
3. Check the atlas: after a long run of different sizes, `GlyphAtlas::len()` should stay bounded and a reset
   should not corrupt the next frame.
