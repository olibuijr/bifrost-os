# PencilKit parity, wave 3

Three gap rows of the PencilKit framework are now modelled in pure Rust in `bifrost-ui`. Each new module is a
pure data model: capturing touch input, drawing the strokes and the tool palette still belong to the shell and apps.
The existing shell editor (`springboard/src/shell/markup.rs`) and the Freeform app are not rewired onto these models
in this wave.

## What was built

- `springboard/ui/src/drawing.rs` (`pub mod drawing`): the drawing model.
  - `Drawing` holds strokes in drawing order, with `bounds`, `transform`, `push`, `stroke_mut` and `from_strokes`.
  - `Stroke` holds an `Ink` and the `StrokePoint`s it passes through, with `bounds` and `transform`.
  - `StrokePoint` keeps location, time offset, size, opacity, force, azimuth and altitude.
  - `Drawing::to_text` and `Drawing::from_text` save and reload a drawing as text. The format is `bifrost-drawing 1`,
    then one `stroke` line and one `point` line per point. Floats keep their exact values. Malformed input, a
    non-finite number or a point before any stroke is an error that names the line (`DrawingError`).
- `springboard/ui/src/lasso.rs` (`pub mod lasso`): lasso selection over a `Drawing`.
  - `Lasso` collects the loop with `add` (NaN and infinite points are dropped) and tests `contains` (even-odd rule,
    so a concave loop works). A loop with fewer than three points encloses nothing.
  - `Lasso::select` returns a `Selection`, the strokes with at least one point inside the loop.
  - `Selection` gives `indices`, `contains_stroke`, `bounds` and `transform` (moves or scales only the selected
    strokes).
- `springboard/ui/src/lib.rs`: one line each, `pub mod drawing;` and `pub mod lasso;`, after `pub mod markup;`.

## Official API names mirrored

- `PKDrawing`: `init(strokes:)`, `init(data:)`, `bounds`, `strokes`, `dataRepresentation()`, `transform(using:)`,
  `transformed(using:)`, `image(from:scale:)` (the image part is not built here).
  Source: https://developer.apple.com/documentation/pencilkit/pkdrawing-swift.struct
- `PKStrokePoint`: `init(location:timeOffset:size:opacity:force:azimuth:altitude:)`, and the properties `location`,
  `timeOffset`, `size`, `opacity`, `force`, `azimuth`, `altitude`.
  Source: https://developer.apple.com/documentation/pencilkit/pkstrokepoint-swift.struct
- `PKLassoTool`: `init()`, the selection of content on a canvas. Source:
  https://developer.apple.com/documentation/pencilkit/pklassotool-swift.struct
- `PKStroke` and `PKInk` are mirrored in shape (`Stroke`, `Ink`) because the drawing model needs them. Their official
  pages were not fetched through the DocC endpoint in this wave (see the notes below).

Where the official pages do not state a rule, the code states its own:

- Lasso selection rule: a stroke is selected when at least one of its points lies inside the loop. The official page
  says the lasso "selects content" and does not give the rule, so this is a BifrOSt choice.
- Transform: an affine transform from `springboard/ui/src/affine.rs` is applied to point locations only. Sizes,
  times and stylus readings are kept.

## REA evidence

- `ev_18bf587974ea492f1250d70c8873630b4d75c86112ecba4ab737b31cf4a9179d`: PencilKit framework landing page
  (`https://developer.apple.com/documentation/pencilkit`).
- `ev_0225f0cfb7bfeb4ab74acd65c1f288b2f60b81281d90b2f5f05093ae68d0dfd2`: `PKDrawing` page.
- `ev_bc7b8a84a18ffbfcb2f8ec4ea4832dae7f62adf4738162e0c97e243dfe29d0f4`: `PKLassoTool` page.
- `ev_6bcdfeb4308df0388a45a6fa2f0e812cf586b81c7384649684e1290caf9b7cd7`: `PKStrokePoint` page.

Notes on the sources: `python3 -I tools/parity/docc.py pencilkit/pkdrawing` (and the same for `pklassotool`,
`pkstrokepoint`, `pkstroke`) returned HTTP 404 from the DocC endpoint, so the symbol pages were read through REA only.
The framework collection page (`docc.py pencilkit`) worked.

## Matrix rows closed

Rows are from `docs/parity-matrix/data/pencilkit.json`. The new status is `partial` for each, because the models have
no shell or app wiring yet.

| Framework | Feature (as in the matrix) | Previous status | New status | BifrOSt path |
|---|---|---|---|---|
| PencilKit | Drawing model (PKDrawing) and save as data | gap | partial | springboard/ui/src/drawing.rs |
| PencilKit | Pressure, tilt and stylus input | gap | partial | springboard/ui/src/drawing.rs (StrokePoint force, azimuth, altitude) |
| PencilKit | Lasso selection and transform (PKLassoTool) | gap | partial | springboard/ui/src/lasso.rs |

## Tests

- `drawing.rs`: 7 tests. Stroke and drawing bounds from hand-worked boxes, the union helper, transform (scale 2 then
  translate (1,-1) maps (3,4) to (7,7)) keeping stylus data, an exact text round trip, rejection of a bad header and
  of a point before any stroke, and rejection of NaN, a short point line and an unknown ink kind.
- `lasso.rs`: 7 tests. A square loop's interior and exterior, a U-shaped loop whose notch is outside, the fewer-than-
  three-points case, the any-point selection rule (selected indices `[0, 2]`), selection bounds `(-5, 2, 10, 3)`, a
  transform that moves only the selected stroke, and dropped non-finite loop points.

Tests were not run in this wave, as the task forbids builds.

## Not done

- Bezier path import, stroke slicing and animation, handwriting recognition, Scribble, and the system tool picker
  remain gaps.
- `PKDrawing.image(from:scale:)` is not built, because rendering belongs to the render crate.
- The shell markup editor and the Freeform app keep their own stroke code.
