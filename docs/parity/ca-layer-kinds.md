# Layer content kinds (Core Animation slice)

Nothing here has been compiled or run by the author (rule: the lead builds). Every row below needs a build, the unit
tests and a Midget check before it moves to Parity.

Shared pieces:

- `springboard/ui/src/raster.rs` (new, `bifrost_ui::raster`): polygon scan conversion to an 8-bit coverage mask
  (non-zero and even-odd, 4 sub-scanlines with exact horizontal coverage) and a stroker (butt, round, square caps;
  miter, round, bevel joins; miter limit via `stroke_style::effective_join`). Used by the shell and the SDK.
- `springboard/sdk/src/layer_kinds/` (new): one self-contained content type per kind, each plain data plus a
  `render(...) -> Image` rasteriser. A future `Layer` holds one of these. `raster.rs` there adds compositing, blits and
  bilinear sampling on premultiplied images.
- `springboard/sdk/src/view/layer_views.rs` (new): `ShapeView`, `GradientView`, `TextLayerView`, `ReplicatorView`,
  `EmitterView`, `TiledView`, `ScrollLayerView`. Each caches its rendered image and re-renders on content, size or scale
  change. Tests: `view/tests_layer_views.rs`.
- Shared-file edits: `view/mod.rs` gained `View::clips_horizontally` (default false; the child clip in `draw_subtree`
  uses the frame's columns when it is true), plus the `mod`/`pub use`/test `mod` lines.

## Shape layer

- API: path, fill colour, fill rule (non-zero, even-odd), stroke colour, line width, cap, join, miter limit, dash
  pattern and phase, stroke start and end.
- Paths: `ShapeContent` in `layer_kinds/shape.rs`; model `ui/src/shape.rs` (`Path::trimmed`, `Path::dashed`),
  `ui/src/stroke_style.rs`; `ShapeContent::from_curves` (ui `path::Path`) and `from_bezier` (render `BezierPath`) flatten
  curves. View: `ShapeView`.
- Done: fill, stroke with all caps and joins, miter limit, dashing, stroke range, hit test (`contains`), bounds.
- Midget: antialiasing quality at 1.5625 scale, stroke range animation cost.

## Gradient layer

- API: axial, radial, conic; colours, locations, start and end point.
- Paths: `layer_kinds/gradient.rs` over `ui/src/gradient.rs`; view `GradientView`.
- Done: all three types in unit coordinates (radial is elliptical in a non-square box), evenly spaced colours when
  locations are absent or the wrong length, 256-entry premultiplied ramp.
- Midget: banding on 8-bit ramps.

## Text layer

- API: string or attributed string, font size, colour, wrapped, alignment (natural, left, right, center, justified),
  truncation (none, start, middle, end).
- Paths: `layer_kinds/text.rs` (`layout_lines` is a pure breaker, `truncate` the ellipsis fitter); view `TextLayerView`.
- Done: wrapping at spaces and inside long words, newlines, truncation of long lines and of the last visible wrapped
  line, justification of non-last wrapped lines, attributed runs (bold, colour, underline, strikethrough, size).
- Not done: natural alignment is left (no right-to-left mirroring); wrapping of attributed text measures with the base
  weight and size, so bold or larger runs can overflow a line slightly; no per-glyph ligature or kerning control.

## Replicator layer

- API: instance count, delay, transform, colour and per-instance colour offset.
- Paths: `layer_kinds/replicator.rs` over `render/src/replicator.rs`; view `ReplicatorView` (`ReplicaSource`).
- Done: per-instance 2D affine about an anchor, translation in points, colour tint and offset, per-instance delay passed
  to an animated source (`render_with`).
- Not done: the z column of the transform is ignored (no perspective); no blue/green/alpha animation of the offsets.

## Emitter layer and cells

- API: birth rate, lifetime, velocity, emission angle and range, spin, scale, colour ranges, render modes, shapes.
- Paths: `layer_kinds/emitter.rs` (cells reuse `render::emitter::Cell` for the kinematics); view `EmitterView`.
- Done: shapes (point, line, rectangle, circle) and modes (points, outline, surface, volume), acceleration, scale and
  colour speeds and ranges, spin, cell shapes (circle, square, image), draw orders (unordered, oldest first/last, back
  to front, additive), seeded and deterministic, warm-up.
- Not done: nested cells (sub-emission), 3D shapes and emission latitude, per-cell birth/lifetime change over time.

## Scroll layer

- API: scroll mode (none, vertical, horizontal, both), scroll to point, scroll to rect.
- Paths: `layer_kinds/scroll.rs` (model), view `ScrollLayerView` (drag, fling and rubber band per axis through
  `bifrost_ui::scroll::Scroller`, indicators on both axes). The older vertical-only `ScrollView` is kept as it is; its
  only users are its own tests.
- Midget: touch feel on a diagonal drag, indicator placement.

## Tiled layer

- API: tile size, levels of detail, async tile drawing.
- Paths: `layer_kinds/tiled.rs` (`TileGrid`, `TileCache`, `TileSource`, `TiledContent`, sync or worker-thread drawing);
  view `TiledView`.
- Done: tile geometry per level (with a magnification bias), visible-tile selection, bounded cache, worker thread with
  `poll`, edge tiles cut to size.
- Not done: no cross-fade of arriving tiles, no coarser-level placeholder while a fine tile is pending.

## Adoption (owner addendum)

- Migrated: `shell/music/icons.rs::fill_poly` now uses `bifrost_ui::raster` (antialiased by exact span coverage).
- Deleted: the duplicate private `fill_poly` in `shell/control_center.rs` (it calls `icons::fill_poly`).
- Left alone: `apps/freeform-desktop/src/style.rs::fill_polygon` (non-antialiased, own pixel format, pixel-exact tests).
- No app manifests changed.
