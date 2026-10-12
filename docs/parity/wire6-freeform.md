# Wire 6: Freeform (1.6.0) wires a pencil ink and soft shadows

Branch `wire6/freeform`, on top of main (release 0.5.43). Freeform is the native app at
`springboard/apps/freeform-desktop/`. The change touches only that folder plus this note. Nothing was built or run
(the lead builds once). Host tests were written by hand, not run.

This round is smaller than the focus asked for. Three of the four focus items are blocked by the SDK boundary, not by
missing UI (see "Not wired and why"). Only the pencil tool and the shadow blur are wired.

## What is now wired

| Matrix slug | Feature (as written in the matrix) | BifrOSt path | How a user reaches it |
|---|---|---|---|
| pencilkit | Pen, marker, pencil and eraser tools (PKInkingTool, PKEraserTool) | `springboard/ui/src/drawing.rs` (`InkKind::Pencil`), used by `freeform.rs` (`Tool::Pencil`, `pen_ink`) | Toolbar: Pencil, beside Pen in the first segmented group. Draws a line half the pen's width in the pencil ink |
| appkit-graphics-printing | Drawing: Bezier paths, gradients, shadows | `springboard/ui/src/layer_style.rs` (`Shadow::radius`), `springboard/apps/freeform-desktop/src/style.rs` (`blur_taps`, `paint_with_blur`) | Lasso strokes, then Inspector > Shadow: On. The shadow now spreads 3 pt around its edge (it was a hard copy) |

### Details

- **Pencil tool.** `Tool::Pencil` is a new case in the app's tool enum. Its ink is `InkKind::Pencil` from the drawing
  model, its width is `SPACE_XS` (4 pt, half the pen's 8 pt). It paints at full opacity, like the pen, and gets the
  crosshair cursor. Toolbar id `BAR_PENCIL` (15). The pencil is stored in the saved drawing as "pencil" ink, the name
  the drawing model already writes. Undo, Delete, Copy and Arrange need no change: they work on any stroke.
- **Soft shadow.** `style.rs` draws the shadow as nine copies: the centre, then eight points on a ring of the blur
  radius. Each copy takes an equal share of the shadow opacity (0.35 / 9), so the total stays the model's opacity.
  The radius comes from `layer_style::Shadow::radius` (3 pt). A radius of zero gives the old hard copy (kept in the
  tests). This is a sampled blur, not a Gaussian: it is smooth enough at 3 pt, and it is not the reference's filter.

## Not wired and why

- **Blend modes** (`springboard/render/src/blend.rs`, `Mode`, `blend_channel`; the matrix row
  core-image "Composite operations and blend modes"). The model lives in `bifrost-render`, which the SDK does not
  re-export (`springboard/sdk/src/lib.rs` `kit` re-exports only `gfx`, `font`, `ui`, `cache`, `clipboard`, `pixel_layout`,
  `appearance`). Freeform depends on `bifrost-app`, `bifrost-core` and `springboard`, not on `bifrost-render`. Wiring
  it needs one `pub use bifrost_render::blend;` in `springboard/sdk/src/lib.rs`, outside this folder. Then the
  inspector gets a Blend row (Normal, Multiply, Screen) and `paint` composites each channel with `blend_channel`.
- **Colour picker with colour-space swatches.** The only reachable colour-space model is
  `springboard/ui/src/surface_colour.rs` (`ColorSpace`: sRGB, linear sRGB, Display P3, gray). It tags a shared buffer,
  it does not convert colours, so swatches tagged with a space would show a label and nothing more. The conversion
  model is `springboard/render/src/colorspace.rs` (`Space`, `Transform`), again in `bifrost-render` and not exported to
  apps. Needs the same SDK re-export. Not done.
- **Symbol shapes from the symbol catalogue.** `bifrost_app::glyphs` (59 names) draws a glyph straight onto a
  `Surface` and has no outline. Freeform keeps its drawing as strokes, so a symbol cannot become a stroke or a fill
  without a path for each glyph. Not done. The glyph names could label the inspector later, but that is not the
  focus of this model.
- **Pen tool picker as a system palette** (`PKToolPicker`, matrix row pencilkit "Tool picker"). The toolbar group is
  the picker. No system palette object is wired. Left as it was.
- **Stylus pressure** (`StrokePoint::force`, `from_device`): Freeform reads position only. Not in this round.

## How to verify on Midget

1. Build and publish as usual, then `bifrost pc midget ota` until `phase=accepted`. Freeform 1.6.0 needs system
   0.5.42 or later (unchanged).
2. Open Freeform with `bifrost pc midget open-app is.olibuijr.freeform.desktop`.
3. Press Pencil in the toolbar (the group now reads Pen, Pencil, Marker, Eraser, Lasso, Rectangle, Oval). Draw a line
   with a pen and a line with the pencil: the pencil line is thinner and the cursor is a crosshair.
4. Draw a rectangle, choose Lasso and select it, press Inspector, then Shadow: On. The shadow is a soft edge, not a
   hard duplicate.
5. Undo once: the shadow goes. Delete removes the selected stroke with its look.
6. Check Light and Dark with `bifrost pc midget appearance light|dark`.
7. Press controls with `bifrost pc midget tap-window Freeform X Y` (window-local pixels).

## REA evidence

- No REA capture was made in this round (the wiring used the matrix notes and the official symbol page via docc).
- docc: `python3 -I tools/parity/docc.py coregraphics/cgblendmode` read the blend-mode constants (normal, multiply,
  screen, overlay, darken, lighten, colour dodge and burn, soft and hard light, difference, exclusion, hue, saturation,
  colour, luminosity). Recorded for the blend row that is not wired yet.
- docc: `python3 -I tools/parity/docc.py pencilkit/pkinkingtool` returned HTTP 404 (wrong page path). Not used.

## Remaining gaps

- Blend modes, colour-space picker and symbol shapes: blocked on the SDK re-exports above (blend, colorspace) and on a
  glyph outline (symbols). Not wired.
- Shadow blur is a sampled blur of nine copies, not the reference's blur filter. Its radius is fixed at 3 pt, not a
  user setting.
- The pencil has no texture and no opacity of its own: the drawing model's ink carries no opacity, and the pencil is
  painted solid.
- Looks are not saved with a drawing (unchanged from 1.5.0).
- `tests/compliance.rs`: the toolbar gains the Pencil button, so the lead must bless the compliance snapshot once with
  `BIFROST_BLESS=1`.
- `springboard/Cargo.lock` still names `bifrost-freeform` at its old version (outside this folder, not edited). The
  next cargo build updates it.
- Not tested on Midget yet, so the matrix rows stay `partial` until a Midget run is recorded.

## Matrix rows (for the lead to update in docs/parity-matrix/)

| Slug | Feature (as written in the matrix) | Suggested status | BifrOSt path |
|---|---|---|---|
| pencilkit | Pen, marker, pencil and eraser tools (PKInkingTool, PKEraserTool) | partial (Midget pending) | springboard/ui/src/drawing.rs, springboard/apps/freeform-desktop/src/freeform.rs |
| appkit-graphics-printing | Drawing: Bezier paths, gradients, shadows | partial (Midget pending) | springboard/ui/src/layer_style.rs, springboard/apps/freeform-desktop/src/style.rs |

Rows not moved: core-image "Composite operations and blend modes" (SDK re-export needed), core-graphics "Colour and
colour spaces (CGColor, CGColorSpace)" (SDK re-export needed), symbols "Symbol-based images (SF Symbols catalogue)"
(glyph outlines needed), pencilkit "Tool picker (PKToolPicker)" (no system palette object).

## Files changed

- `springboard/apps/freeform-desktop/src/freeform.rs`: `Tool::Pencil`, `BAR_PENCIL`, `ink_width`, `id_target` and
  `target_id`, the toolbar item, the cursor, `pen_ink`, the stroke start, and the host test
  `the_pencil_is_a_thin_pencil_ink_in_the_pen_group`.
- `springboard/apps/freeform-desktop/src/style.rs`: `SHADOW_BLUR_PT`, `shadow_model(blur_pt)`, `blur_taps`,
  `paint_with_blur` (`paint` calls it with the blur radius), and the host tests
  `a_blur_radius_gives_nine_copies_and_none_gives_one` and `a_blurred_shadow_reaches_past_the_hard_shadow_edge`.
- `springboard/apps/freeform-desktop/Cargo.toml`, `manifest.toml`: version 1.6.0.
- `springboard/apps/freeform-desktop/CHANGELOG.md`: 1.6.0 entry.
- `docs/parity/wire6-freeform.md`: this note.

## Checks

- `python3 tools/check-names.py`: no hits.
- Hand-worked values for the blur test: a 2 pt line at y = 30 casts its hard shadow at y = 34 (offset 4). The blur
  copy at (0, 3) lies at y = 37, so pixel (15, 37) is painted only by the blur. Each copy paints alpha
  round(255 * 0.35 / 9 * 1) = 9 at full cover, and mix(255, 0, 9) = 246, which is not white and is above 200.
  The hard shadow (radius 0) leaves the same pixel white. The nine taps are the centre and the ring at 45 degree
  steps, so the second tap is (3, 0).
