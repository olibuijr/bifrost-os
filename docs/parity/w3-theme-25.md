# Wave 3, theme-25: drawing model and save as data

Closes the drawing-model, stylus, stroke-editing and PDF annotation gap rows of the PencilKit and PDFKit
frameworks in the parity matrix. The shared code is pure std Rust in the `bifrost-ui` crate. Nothing is wired into
the shell's markup editor, Freeform or the PDF reader yet, so the rows are `partial`, not `parity`.

## What was built

- `springboard/ui/src/drawing.rs` (module `drawing`):
  - `Drawing`: a list of strokes. Save and load as bytes (`data_representation`, `from_data`) in a versioned
    little-endian format (magic `BDRW`, version 1). Loading rejects a bad magic, another version, an unknown tool,
    truncated or over-long input and any non-finite number.
  - `Stroke` and `StrokePoint`: timed points with position, time, force, altitude and azimuth, plus a per-stroke
    transform. `StrokePoint::from_device` scales a raw force by the device maximum and clamps it to 0 to 1.
  - `Stroke::width_at`: BifrOSt's own force rule. The width runs from 25 percent of the ink width at zero force to
    the full width at full force. This is our rule, not one copied from the reference SDK.
  - `Stroke::point_at` and `Stroke::slice`: interpolation along a stroke by distance, and slicing it between two
    distances. This is slicing and interpolation only. There is no blending and no animation.
  - `Drawing::lasso` and `Drawing::transform_strokes`: select strokes that have a point inside a polygon, and
    transform only the selected strokes. This is a stroke-level lasso with no per-point selection.
  - `Drawing::from_path`: flattens a path with the existing `path` module and turns each subpath into a stroke.
  - `Drawing::bounds`: the bounds of the points in drawing space, after the transforms.
- `springboard/ui/src/annotation.rs` (module `annotation`):
  - `Subtype`: the thirteen annotation kinds named in the reference SDK.
  - `Annotation`: subtype, page-space bounds, contents, colour and an optional drawing. `Annotation::ink` takes its
    bounds from the drawing.
  - `PageAnnotations`: stacking order, ids that are never reused, `annotation_at` hit testing (topmost first, half
    open bounds) and a page-wide show or hide flag.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each module.

No platform names are used in the code. The reference names appear only in this document.

## Reference API mirrored (official documentation)

- PencilKit: `PKDrawing` (strokes, `init(strokes:)`, `dataRepresentation`, `bounds`, `transform`, `append`),
  `PKStroke` (ink, transform, points), `PKStrokePoint` (location, timeOffset, force, altitude, azimuth),
  `PKInk`, `PKLassoTool`, and the Bezier import and stroke-animation articles.
  DocC: `https://developer.apple.com/documentation/pencilkit/pkstrokepoint-swift.struct`,
  `https://developer.apple.com/documentation/pencilkit`
- PDFKit: `PDFAnnotation` (`init(bounds:forType:withProperties:)`, `bounds`, `contents`, `color`, `draw`),
  `PDFAnnotationSubtype`, `PDFPage` `addAnnotation` / `removeAnnotation` / `annotation(at:)` and
  `displaysAnnotations`.
  DocC: `https://developer.apple.com/documentation/pdfkit/pdfannotation`,
  `https://developer.apple.com/documentation/pdfkit/pdfannotationsubtype`

The DocC path `pencilkit/pkdrawing` returned 404. The `PKDrawing` page was read through its rendered page instead
(see the REA evidence below).

## REA evidence (rendered official pages)

- PencilKit landing: `ev_40a0081bf0c9c4ddab5aec409bd5fc8cdf8fdc7268b60c1a370badd1d80e1818`
- PKDrawing (`pencilkit/pkdrawing-swift.struct`): `ev_d88f1eddf59838b5689b101a3e80e4ef43479b661965f3795f9e08edd3ad4754`
- PDFKit landing: `ev_2b0cc8df76e436c1140e658fd2d501abf40bb3ac9a47d41302c0b47cc05b5188`

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| pencilkit | Drawing model (PKDrawing) and save as data | partial | springboard/ui/src/drawing.rs (Drawing::data_representation, Drawing::from_data) |
| pencilkit | Pressure, tilt and stylus input | partial | springboard/ui/src/drawing.rs (StrokePoint::from_device, Stroke::width_at); no device wiring |
| pencilkit | Stroke slicing, animation and blending | partial | springboard/ui/src/drawing.rs (Stroke::point_at, Stroke::slice); no animation or blending |
| pencilkit | Bezier path import (PKStroke from paths) | partial | springboard/ui/src/drawing.rs (Drawing::from_path) |
| pencilkit | Lasso selection and transform (PKLassoTool) | partial | springboard/ui/src/drawing.rs (Drawing::lasso, Drawing::transform_strokes) |
| pdfkit | Annotations | partial | springboard/ui/src/annotation.rs (Annotation, PageAnnotations) |

## Not closed

- Handwriting recognition, Scribble on non-text views and the PencilKit tool picker need an input path or a
  recogniser, neither of which exists here.
- The existing PencilKit "Stroke and point model" and "Ink type" rows gain the model fields (time, force, altitude,
  azimuth), but the shell still keeps its own stroke list. Those rows are not claimed here.
- PDF annotations are not read from or written to PDF files. The PDF engine in `springboard/src/shell/books` is
  unchanged.

## Verification

Not built or run, as the task rules require. Each module has unit tests whose expected values were worked out by hand
from the rules in the code (byte size 211 for the two-stroke sample, length 11 and interpolated points, slice output,
width scaling, lasso selection, bounds, path import and annotation hit testing).
