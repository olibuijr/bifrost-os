# Wiring the parity drawing models into Freeform (1.3.0)

Freeform (`springboard/apps/freeform-desktop`, `is.olibuijr.freeform.desktop`) now uses the pure models from
`bifrost-ui` (`bifrost_app::controls`): the drawing and stroke model (`drawing`), lasso selection (`lasso`), the path
model (`path`, the Bezier element type). Nothing new was written as a model; `ink.rs` is a small adapter that paints
strokes into the pixel canvas and builds shape outlines.

## What is now wired

| Matrix slug | Feature (`docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status |
|---|---|---|---|
| `pencilkit` | Lasso selection and transform (PKLassoTool) | `springboard/ui/src/lasso.rs` via `freeform.rs` (`Lasso`, `Selection::transform`, `bounds`) | wired: loop, select, move, delete, undo |
| `pencilkit` | Stroke and point model (PKStroke, PKStrokePath, PKStrokePoint) | `springboard/ui/src/drawing.rs` (`Drawing`, `Stroke`, `Ink`, `StrokePoint`) via `freeform.rs` | wired: every pen, marker and eraser stroke is recorded |
| `pencilkit` | Undo for drawings | `freeform.rs` `undo` (canvas and drawing together) | wired for Freeform |
| `paperkit` | Shapes and text boxes (shapes part) | `springboard/ui/src/path.rs` (`add_rect`, `add_ellipse_in`) and `drawing.rs` (`Drawing::from_path`) via `ink.rs` | wired: rectangle and oval; text boxes not wired |
| `pencilkit` | Bezier path import (PKStroke from paths) | `drawing.rs` `Drawing::from_path` via `ink.rs::shape_drawing` | wired for the oval outline (cubic curves flattened to points) |
| `appkit-graphics-printing` / `core-graphics` | Drawing: Bezier paths; Path construction (CGPath) | `springboard/ui/src/path.rs` | wired as the shape path; general user-drawn curves are not |

## How a user reaches it

Open Freeform (Applications or the dock). The toolbar's first group now has six segments:

- Pen, Marker, Eraser: unchanged in feel. Each stroke is recorded as a stroke as well as drawn.
- Lasso: drag a loop around strokes. Strokes with any point inside the loop are selected and outlined in the tint.
  Drag inside the outline to move the selection. The Delete button in the second group (enabled while something is
  selected) removes it. Selecting another tool clears the selection.
- Rectangle, Oval: drag out a shape. Release to draw it in the current colour. A tap (no real drag) draws nothing.
- Undo restores the picture and its strokes together, so it also undoes a delete, a move or a shape.
- Clear empties both the picture and the strokes.

Everything is laid out by `widgets::toolbar`. Each new tool is a `ToolbarItem::button` in the segmented group, so the
toolbar's kit look and the published controls (`Freeform::controls`) are unchanged in kind.

## How to verify on the test PC (Midget)

1. Publish and update: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Freeform 1.3.0
   needs system 0.4.84 or later (unchanged).
2. Open Freeform with `bifrost pc midget open-app is.olibuijr.freeform.desktop`.
3. Draw two strokes with the pen, far apart. Choose Lasso and drag a loop around one of them. Expect the outline.
4. Drag inside the outline with `bifrost pc midget tap-window` (or a swipe): the stroke moves with the finger.
5. Press Delete: the stroke goes; Undo brings it back. Repeat with Rectangle and Oval (drag, release).
6. `bifrost pc midget screen /tmp/freeform.png` and compare with the Golden Gate look (toolbar as a segmented group,
   the tint as the only accent).
7. Check Light and Dark (`bifrost pc midget appearance light|dark`): the board stays paper; the outline uses the tint.

Host tests: `cargo test` in `springboard/apps/freeform-desktop` covers `ink.rs` (painting, marker and eraser ink,
trail spacing, shapes and nudge) and the app model (pen stroke and undo, lasso select and delete, selection move,
rectangle and oval, tap cancel). Not built in this round; the lead builds once.

## Known differences and gaps

- Marker strokes replayed after a lasso move use a plain translucent blend, not the live "keep the stronger mix"
  rule, so an overlapping marker stroke can look slightly lighter after it is moved.
- Snapshots: undo keeps the full picture plus strokes, up to 12 steps. Large boards use more memory than before.
- `compliance.rs` compares the app with a golden snapshot. The toolbar changed (three tools added), so the golden
  must be blessed (`BIFROST_BLESS=1`) by the lead after the build.
- Gradients (`springboard/ui/src/gradient.rs`, CGGradient / CAGradientLayer) are not wired. The stroke model has no
  fill, so there is nowhere to paint a gradient yet. Gap.
- Pressure, tilt and stylus data (`StrokePoint.force`, `azimuth`, `altitude`) are recorded as zero for finger input;
  the model keeps them but nothing reads them yet.
- Text boxes, links, loupes and images in markup (PaperKit) are not wired here; only shapes are.
- Lasso selects by point, not by stroke geometry; there is no scale handle, only move and delete.
- Curved user strokes are still polylines (the pen samples points); Bezier paths are used for the shape outlines only.

## REA evidence (reference behaviour)

Official documentation was read with `python3 -I tools/parity/docc.py` and the rendered page with
`tools/parity/rea-page.sh`:

- `ev_197969834935dc901ff39fcb366426d4b7bbe2ab6c62e6bc22317d1eda70dcaa`: PKLassoTool (docc
  `pencilkit/pklassotool-swift.struct`): "A tool for selecting stroked lines and shapes in a canvas view"; the lasso
  selects content in a canvas and the selection can then be moved.
- `ev_ca8a43cbb534dec3e3a62e4bc52d95a224ccb775da70f62b95d6db38a155500c`: NSBezierPath (docc `appkit/nsbezierpath`):
  "Paths consist of straight and curved line segments joined together"; `init(ovalIn:)` for an oval path.

Parity rows are not edited here (`docs/parity-matrix/` is out of scope for this round). The lead should promote the
rows above after the Midget check, following the bifrost-parity-matrix skill.
