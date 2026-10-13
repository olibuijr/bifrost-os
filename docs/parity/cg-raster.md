# Core Graphics: paths, transforms, scan conversion, stroking (slug cg-raster)

Code: `springboard/render/src/cg/{path,raster,surface}.rs`. Not built or run in this branch (the lead builds).

| Reference rows | BifrOSt path | Done | Needs Midget checking |
|---|---|---|---|
| Path construction (move/line/quad/cubic/close, lines, rects, rounded rect, ellipse, arc, relative arc, arc-to, add path) | `cg::path::Path` | all | arc direction convention (`clockwise` is the sense of the angle) |
| Path queries (bounds, control-point bounds, contains with rule, is empty, is rect, current point, equality, apply/iterate) | `Path::bounds`, `control_bounds`, `contains`, `is_rect`, `iter`, `apply`, `PartialEq` | all | none |
| Copies (transform, flatten, stroke, dash, separate components, intersects) | `transformed`, `flattened`, `copy_by_stroking`, `copy_by_dashing`, `separate_components`, `intersects` | done | none |
| Boolean path ops (union, intersection, subtract, symmetric difference, normalize, line intersect/subtract) | not done | gap | n/a |
| Affine (make, identity, concat, invert, translate/scale/rotate both as constructors and as appliers, apply to point/size/rect, equality, decompose) | `cg::path::Affine` | all | none |
| Fill, non-zero and even-odd, anti-aliased, clip by coverage | `cg::raster::fill`, `Coverage`, `FillRule` | exact horizontal spans over 8 sub-scanlines | quality on HiDPI |
| Stroke: caps, joins, miter limit, dash and phase | `cg::raster::stroke_path`, `StrokeStyle` | all | none |
| Line, ellipse, arc fill/stroke on the pixel surface | `gfx::Surface::{stroke_line, fill_ellipse, stroke_ellipse, stroke_arc, fill_arc, fill_path, stroke_path}` (in `cg/surface.rs`) | done | visual check |

Moved and deleted: `ui/src/affine.rs`, `ui/src/path.rs`, `ui/src/stroke_style.rs`, the path half of `ui/src/shape.rs` (now only the `Shape` enum). The scan converter and stroker reuse the logic of the wave-1 `ui/src/raster.rs`.

The font engine's private flattening (`render/src/font.rs`) is left alone: it feeds its own exact-area glyph rasteriser and is not a straight swap.
