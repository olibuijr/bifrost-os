# G1 `canvas`: Canvas for Surface, DisplayList recorder

Part of the graphics core in `docs/graphics-architecture.md` (wave G1). This round implements the drawing vocabulary
(`springboard/render/src/canvas.rs`) on the CPU `Surface`, and the recorder that the GPU executor (`bifrost-2d`)
replays (`springboard/render/src/display_list.rs`). No existing caller changes: every current `Surface` method is
still there with the same signature and behaviour.

## What is implemented

### `canvas.rs`

- **Transform math.** `Transform2D` gained `translate`, `scale`, `mul` (`self · o`, `o` applied first), `apply`,
  `inverse` (`None` for singular) and `is_translation`. Free functions `bbox_of` (device box of a rectangle under a
  transform) and `intersect`.
- **State.** `CanvasState` holds the current transform, the clip stack, the opacity and their restore stacks.
  Unbalanced pops are ignored. Each clip keeps its own transform and inverse, so a clip under a rotation is still a
  rounded rectangle in its own space. `clip_coverage` is the product of the clips' anti-aliased rounded-rectangle
  masks (`gfx::rrect_coverage`). `key()` hashes the state for damage.
- **Surface routing.** `impl Canvas for Surface` has three routes, chosen per call in `run`:
  1. Plain state (translation, no clip, opacity 1): the existing `Surface` method is called with the offset applied.
  2. Translation with a clip or opacity: the op is drawn into a scratch `Surface` covering its device box clipped to the
     clips, then composited pixel by pixel with `clip_coverage * opacity` (source-over).
  3. Any other affine transform: the op is drawn in local space into a scratch surface covering its local box, then
     device pixels are mapped back through the inverse and sampled nearest-neighbour.
- **Per-op implementations.** `fill_rect`, `fill_rrect`, `stroke_rrect` and `image` / `image_tinted` call the existing
  `Surface` methods. `linear_gradient` evaluates a premultiplied stop ramp per pixel inside the rounded rectangle.
  `shadow` builds the rounded-rectangle mask, blurs it with `Image::blur` (three box passes, radius `round(blur/2)`)
  and composites it in the colour. `path` flattens curves (16 segments per quadratic, 24 per cubic), fills with
  non-zero or even-odd winding on 4 sub-scanlines with exact horizontal coverage, and strokes with round joins and caps
  by distance to segments (4x4 samples per pixel). `material` calls `Surface::glass_rrect` on the device box, with the
  backdrop copied into the scratch and the result mixed back by coverage (so a clipped material never draws over
  unclipped pixels). `text` renders the run with the font module and blits it.
- **Font faces.** `TextRun.face` is an id from `register_face(&'static [u8])` (the data address). Parsed fonts are cached
  per thread. An unknown face draws nothing.

### `display_list.rs`

- `DisplayList` records every `Canvas` call as an `Op`, with the same `Op` variants as before. Each drawing op stores
  its device bounds (local extent transformed, cut to the clip box) in `bounds`, and the hash of its state in the new
  `states` field. State ops have empty bounds.
- Local extents: shadows include the blur reach (`3·round(blur/2) + 2 + |offset|`), paths include the stroke half-width,
  text uses the measured advance and ascent-plus-descent (with a generous fallback for an unregistered face).
- Images are captured once per id (`images`, an `Arc` copy). `captured(id)` gives the pixels back.
- `replay(&mut dyn Canvas)` re-emits every op in order, so a list replayed into a `Surface` matches direct drawing.
- `bounds_changed(previous)`: a positional diff over `(op, bounds, state)` with a resync window of 16 ops, so an
  insertion or removal damages only the ops that moved. A size change damages the whole list. An op drawn with a
  different opacity, transform or clip is damage even when the op itself is unchanged.

## How it maps to the contract

| Contract item | Status |
|---|---|
| `Canvas` trait, method list and signatures | unchanged |
| `Surface` implements `Canvas` with its CPU methods | done; plain-state calls go straight to them |
| `DisplayList` records `Op`s with device bounds | done |
| Images captured by id, `Arc` copy once per list | done |
| `bounds_changed` diff, `replay` | done (see resync note below) |
| Clips: rounded-rect stack, applied by masking | done (scratch and composite) |
| Transforms: translation exact, other affines through an image fallback | done (nearest sampling) |
| Glyph runs through the face registry | done, parsed-font cache per thread |

## Contract changes

Additive only, no signature changed:

- `Transform2D`: new methods `translate`, `scale`, `mul`, `apply`, `inverse`, `is_translation`.
- `canvas`: new items `bbox_of`, `intersect`, `CanvasState`, `ClipEntry`, `register_face`, `face_data`,
  `reset_surface_state`, `surface_state` (crate-private), `with_font` and `render_text` (crate-private),
  `text_extent` and `flatten` (crate-private).
- `display_list::DisplayList`: new public field `states: Vec<u64>` (parallel to `ops`) and a private recorder state.
  Code that builds a `DisplayList` with a struct literal must add `states` (or use `DisplayList::new`).
- `TextRun.face` now means: an id returned by `register_face` (the address of the font data). The skeleton's doc
  already said "the address of its font data"; this round makes that id the lookup key.

## Known limits (todo_left)

1. **Surface state is per thread, not per surface.** `Surface` has no room for a state field without touching every
   struct literal in the apps. The canvas state lives in a thread-local, so a clip pushed on one `Surface` applies
   to every `Surface` the same thread draws until it is popped. Worker threads from `Surface::draw_rows` do not see the
   caller's state. Fix: a `CanvasSurface<'a>` wrapper, or a state field after the apps move to `bifrost-2d`.
2. **Non-translation transforms sample nearest.** No anti-aliasing for rotated or scaled images and shapes. Fix:
   bilinear sampling of the scratch, or the GPU path.
3. **Material.** `saturation` is ignored on the CPU. Under a rotation or scale the material covers the device box of
   its rectangle. With the GPU presenter active (`glass::gpu()`), `glass_rrect` pushes the device rectangle
   unclipped, so a clipped or faded material does not honour its clip there.
4. **Shadows** use three box passes (a Gaussian approximation), not an analytic blur.
5. **Stroke joins and caps** are round only; no miter, no dash. Stroke and path cost is O(pixels x segments x 16) and
   will be slow for large paths. Fine for UI glyphs and icons; the GPU executor should tessellate.
6. **Gradients** are linear only, with no dithering.
7. **Text** is rendered per call (no glyph cache on the CPU side). Positions round to whole pixels.
8. **`bounds_changed`** resyncs within 16 ops only; a longer insertion damages the rest of the list.
9. **Tests were written but not executed in this round.** The task rules forbid `cargo test`, so the tests in
   `canvas.rs` and `display_list.rs` were hand-checked (the expected values are worked out in the test bodies) and
   `cargo check -p bifrost-render --tests` passes. Run `cargo test -p bifrost-render canvas display_list` before
   merging.

## How to verify

Host:

```
cd springboard
CARGO_TARGET_DIR=... cargo test --offline -p bifrost-render canvas display_list
CARGO_TARGET_DIR=... cargo check --offline -p bifrost-render --tests
python3 tools/check-names.py
```

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): this round changes no pixel output, because every caller still uses the
existing `Surface` methods. Confirm with the usual loop: `bifrost pc publish`, `bifrost pc midget ota` until
`phase=accepted`, then `bifrost pc midget screen /tmp/g1.png` and compare with the previous release. The
replay-equivalence test (`replay_into_a_surface_matches_direct_drawing`) is the bridge to the GPU executor: once
`bifrost-2d` executes a `DisplayList`, run the same recorded list through both paths and compare pixels on Midget.
