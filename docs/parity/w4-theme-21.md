# Wave 4, theme 21: scribble on non-text views

Theme 21 covers stroke data and shape styling across PencilKit, HIDDriverKit, Core Text and Core Animation. This
wave closes four of the seven gap rows in the theme. The pure models live in `springboard/ui/src` and need no
build changes beyond one `pub mod stylus;` line in `springboard/ui/src/lib.rs`. Nothing is wired into the shell
or the apps yet, and nothing has been built or tested on the VM or Midget (the wave ran without builds, by rule).

## What was built

- **`springboard/ui/src/shape.rs`** (extended, Core Animation shape layer):
  - `Path::contains(x, y, FillRule)`: point-in-path hit test with the non-zero and even-odd fill rules. Each
    subpath is closed for the test.
  - `Path::dashed(pattern, phase)`: splits a path into dashes along a repeating on/off pattern, with a phase.
    Dashes never join across subpaths. An empty, negative or zero-sum pattern returns the path unchanged.
  - `FillRule`, `LineCap` (butt, round, square), `LineJoin` (miter, round, bevel) and `StrokeStyle` (width, cap,
    join, miter limit). `StrokeStyle::effective_join(angle)` applies the miter limit: a miter reaches
    `1 / sin(angle / 2)` widths, and past the limit it becomes a bevel.
  - Trimming (`trimmed`) and length already existed and map to `strokeStart` and `strokeEnd`.
- **`springboard/ui/src/drawing.rs`** (extended, PencilKit stroke model):
  - `Stroke::length`, `Stroke::slice(from, to)`: the part of a stroke between two length fractions, with interpolated
    ends and every point between them kept.
  - `Stroke::resampled(n)`: `n` points evenly spaced by length.
  - `Stroke::blend(a, b, t)`: blends two strokes for animation. Both are resampled to the longer point count,
    positions and stylus readings are interpolated, the colour is blended per channel, and the ink kind switches at
    the halfway point.
  - `Drawing::from_path(path, tolerance, ink)`: Bezier import. Each subpath of a `path::Path` becomes one stroke,
    with curves flattened to points within the tolerance.
- **`springboard/ui/src/stylus.rs`** (new, digitizer input):
  - `LogicalRange` (`unit` 0..=1, `signed` -1..=1), `StylusProfile` (pressure and tilt ranges), `StylusReport` (one
    raw pen report with touching and eraser flags).
  - `pen_angles(tilt_x, tilt_y)`: altitude and azimuth from two tilt angles. The lean is `asin` of the length of the
    vector of the two sines, so an upright pen gives altitude pi/2 and a flat pen gives altitude 0.
  - `StylusReport::to_point(profile)`: a `StrokePoint` with force, azimuth and altitude, or `None` when the tip is
    not touching.

## Official API names mirrored

- Core Animation: `CAShapeLayer`, `CAShapeLayerFillRule`, `CAShapeLayerLineCap`, `CAShapeLayerLineJoin`, the path,
  `lineDashPattern`, `lineDashPhase`, `lineWidth`, `miterLimit`, `strokeStart` and `strokeEnd`.
- PencilKit: `PKStroke` (its path import, `substroke(range:)` and the stroke point data), `PKStrokePoint`, `PKInk`.
- HIDDriverKit: the stylus element data for position, tilt and twist, as the stylus sample reports it.

## REA evidence

- CAShapeLayer, official page: `ev_ce3cffd39b261f57d780796b8cb77cbcf38fa9f4553415cc1943366ae52d309f`
- PKStroke, official page: `ev_2396ba11b928d64fd3159fdee97f98b43bf35372cf6540a1d66617eb4acc3844`
- Handling Stylus Input from a Human Interface Device, official page:
  `ev_61ab5cc5c475287c5779b0182bef869a68a4d5eb99bd52bd59e0106790eb484c`

## Matrix rows closed (status partial)

| Framework slug | Feature (as in the matrix data) | New status | BifrOSt path |
|---|---|---|---|
| pencilkit | Bezier path import (PKStroke from paths) | partial | springboard/ui/src/drawing.rs (`Drawing::from_path`) |
| pencilkit | Stroke slicing, animation and blending | partial | springboard/ui/src/drawing.rs (`Stroke::slice`, `resampled`, `blend`) |
| hiddriverkit | Stylus and pen input | partial | springboard/ui/src/stylus.rs (`StylusReport::to_point`, `pen_angles`) |
| core-animation | Shape layer (CAShapeLayer, Bezier paths) | partial | springboard/ui/src/shape.rs (`Path::contains`, `dashed`, `StrokeStyle`, `FillRule`, `LineCap`, `LineJoin`) |

Partial, not full: none of these is wired into the shell, the markup editor or the Freeform app. The stylus model
takes already-decoded report values and does not parse a device's HID report descriptor. The shape layer has no
renderer for caps, joins or dashes, and no stroke outline is generated. Blending pairs points by index after
resampling; it does not match strokes by shape.

## Gap rows left open

- pencilkit, "Scribble on non-text views" and "Handwriting recognition (PKStrokeRecognizer)": a recogniser needs
  a trained model or a large rule set. Neither fits a pure, std-only model in this wave.
- core-text, "Line and run layout objects (typesetter, framesetter, line, run, frame)": text layout belongs to the
  font engine. A clean split would need a shared layout API, which was out of scope for this theme.

## Notes for the lead

- `FillRule` in `shape.rs` duplicates the idea of `WindingRule` in `springboard/render/src/bezier.rs`. `ui` does not
  depend on `render`, so the enum is repeated here. Both can be unified later if the crates share a dependency.
- Tests: `shape.rs` (6 new tests), `drawing.rs` (5 new tests), `stylus.rs` (6 tests), each worked out by hand from the
  rule the code implements.
- `python3 tools/check-names.py` passes.
