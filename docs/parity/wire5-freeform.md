# Wire 5: Freeform (1.5.0) wires 2D drawing models into the shape inspector and PNG export

Branch `wire5/freeform`, on top of main (release 0.5.42). Freeform is the native app at
`springboard/apps/freeform-desktop/`. The change touches only that folder plus this note. Nothing was built or run
(the lead builds once). Host tests were written by hand, not run.

## What is now wired

| Matrix slug | Feature (as written in the matrix) | BifrOSt path | How a user reaches it |
|---|---|---|---|
| core-graphics | Affine transforms and transform stack (CGAffineTransform, CTM) | `springboard/ui/src/affine.rs` (`AffineTransform::translate`, `scale`, `rotate`, `then`) | Lasso a shape, then Transform: Turn (15 degrees), Smaller (80 per cent), Larger (125 per cent), each about the selection's middle |
| core-graphics | Gradients (linear and radial, CGGradient) | `springboard/ui/src/gradient.rs` (`Gradient::new`, `color_at`, `linear_t`) | Lasso a rectangle or oval, then Fill: Gradient. It runs left to right from the current colour to the next colour in the palette |
| core-graphics | Path construction and stroking (CGPath, CGMutablePath) | `springboard/ui/src/stroke_style.rs` (`dash_polyline`) | Lasso strokes, then Dash: Dashed. The stroke is cut into dash pieces on a 6 pt on, 4 pt off pattern |
| core-graphics | Bitmap drawing context (CGContext, bitmap contexts) | `springboard/apps/freeform-desktop/src/style.rs` (`Bitmap`), with `ink::paint_disc` | File > Export as PNG (Command-Shift-E). The board is painted into a bitmap of the export size and written as a PNG |
| appkit-graphics-printing | Drawing: Bezier paths, gradients, shadows | `springboard/ui/src/layer_style.rs` (`Shadow`, `effective_opacity`) | Lasso strokes, then Shadow: On. The shadow uses the model's opacity (0.35) and offset (3 pt right, 4 pt down) |

### Details

- **Shape inspector.** `Freeform::edit` and the panel (`draw_inspector`, `inspector_hit`) are new. A toolbar
  button "Inspector" (group 3, before Save to Photos) shows or hides a 220 pt panel on the right. The board
  narrows by the panel's width, and the strokes are painted again when the width changes, so nothing is lost. Rows:
  Fill (None, Solid, Gradient), Stroke width (thin, medium, thick: half, the pen's, twice the pen's width, written
  to the stroke's ink), Dash (Solid, Dashed), Shadow (Off, On), Opacity (100, 50, 25 per cent), Transform (Turn,
  Smaller, Larger). Each row is a kit segmented control. With no selection the controls are greyed and a caption
  says to select with Lasso.
- **Per-stroke look.** `style::Style` (fill, dashed, shadow, opacity, and a `shape` flag) is kept beside the drawing
  model, one entry per stroke at the same index. `sync_styles` keeps the lengths equal. Undo snapshots carry the
  looks, Delete removes them with their strokes, Clear empties them, and Arrange and lasso moves keep the indices.
  The shared model crate is unchanged.
- **Fill.** Only rectangles and ovals (the shapes the Rectangle and Oval tools draw) take a fill. The fill is a
  scanline fill of the closed outline (even-odd, not antialiased). The outline is drawn over it.
- **Export.** `Freeform::export_as_png` writes the whole board through `Bitmap` at the display factor
  (`export_scale`), so the file is as sharp as the screen. The File menu item is `main_menu` in `main.rs` with title
  "File"; the shell bar merges it into its File menu (the same path the Console and Books apps use). It is off until
  the board has a stroke. Save to Photos still exists and writes the same file type.

## How to verify on Midget

1. Build and publish as usual, then `bifrost pc midget ota` until `phase=accepted`. Freeform 1.5.0 needs system
   0.5.42 or later (unchanged).
2. Open Freeform with `bifrost pc midget open-app is.olibuijr.freeform.desktop`.
3. Draw a rectangle and an oval (Rectangle, Oval, drag). Choose Lasso and drag a loop around both.
4. Press Inspector in the toolbar. The panel shows on the right. Press Fill: Solid, then Gradient. Press Dash: Dashed,
   then Shadow: On. Press Opacity: 50%. Each press is one Undo step.
5. With the same selection, press Larger. The shapes grow about their middle. Press Turn: they turn 15 degrees.
6. Choose File > Export as PNG... (the File menu of the bar). The toast says "Exported Freeform-<ms>.png as PNG".
   Check the file in the Photos folder with `bifrost pc midget screen PATH` or a file viewer.
7. Press controls with `bifrost pc midget tap-window Freeform X Y` (window-local pixels). Check Light and Dark with
   `bifrost pc midget appearance light|dark`: the panel uses the sidebar colour and the captions the secondary label.

## REA evidence (official pages, rendered)

- `ev_57dd428dc75967bb742bf3a9099461718dc9fb3d5fcb7f480e754aae70acb89f`: CGAffineTransform
  (developer.apple.com/documentation/coregraphics/cgaffinetransform). The page lists the make, rotation, scale,
  translation, translate, scale and rotate functions.
- `ev_6d42c1fc8a557499b69790d6b5e78f1473fa1ea021291e5c25a7eb437986566d`: CGGradient
  (developer.apple.com/documentation/coregraphics/cggradient). The page states that a gradient is a definition for a
  smooth transition between colours for radial and axial fills.
- `ev_3b8eadc8f3a05e56a9aa64e694d05257a0dd06077f2fec8a15826edd461cf812`: CGContext
  (developer.apple.com/documentation/coregraphics/cgcontext), the drawing context page.
- docc: `python3 -I tools/parity/docc.py coregraphics/cgaffinetransform` and `coregraphics/cggradient` (symbol lists
  read, no extra claims). The shadow and blend mode pages were not read (see the gaps).

## Remaining gaps

- **Blend modes** (CGBlendMode) are not wired. The app paints with straight alpha (`ink::mix`) only.
- **Shadow blur** is not drawn: the shadow is a hard offset copy of the stroke. `Shadow::radius` is zero.
- **Fill is not antialiased** and is linear only. Radial gradients are not offered, and a gradient's colours come from
  the palette, not from a picker.
- **Stroke colour** cannot be changed on a selection (the palette picks the colour of new ink and of the fill).
- **Copy** puts the stroke shapes on the pasteboard, without their looks. The look is not in the Freeform stroke data.
- **Looks are not saved** with a drawing: Freeform has no document file, so they last until the app closes.
- **Tests not run.** `tests/compliance.rs` will differ: the toolbar gains the Inspector button, so the lead must run
  it with `BIFROST_BLESS=1` once.
- **Cargo.lock** still says `bifrost-freeform` 1.4.0 in `springboard/Cargo.lock` (outside this folder, not edited).
  The next cargo build updates it, or run the build with the lock refreshed.
- Not tested on Midget yet, so the matrix rows stay `partial` until a Midget run is recorded.

## Matrix rows (for the lead to update in docs/parity-matrix/)

| Slug | Feature (as written in the matrix) | Suggested status | BifrOSt path |
|---|---|---|---|
| core-graphics | Affine transforms and transform stack (CGAffineTransform, CTM) | partial (Midget pending) | springboard/ui/src/affine.rs, springboard/apps/freeform-desktop/src/freeform.rs |
| core-graphics | Gradients (linear and radial, CGGradient) | partial (Midget pending) | springboard/ui/src/gradient.rs, springboard/apps/freeform-desktop/src/style.rs |
| core-graphics | Path construction and stroking (CGPath, CGMutablePath) | partial (Midget pending) | springboard/ui/src/stroke_style.rs, springboard/apps/freeform-desktop/src/style.rs |
| core-graphics | Bitmap drawing context (CGContext, bitmap contexts) | partial (Midget pending) | springboard/apps/freeform-desktop/src/style.rs (Bitmap) |
| appkit-graphics-printing | Drawing: Bezier paths, gradients, shadows | partial (Midget pending) | springboard/ui/src/layer_style.rs, springboard/apps/freeform-desktop/src/style.rs |

Rows not moved: blend modes (no row was wired), and the shadow blur part of the row above.

## Files changed

- `springboard/apps/freeform-desktop/src/style.rs` (new): `Style`, `Fill`, the painter (`paint`, `paint_strokes`),
  the scanline fill, `Bitmap`, and host tests (`a_solid_fill_covers_the_rows_inside_the_polygon_only`,
  `half_opacity_leaves_a_mid_grey_pixel`, `a_dashed_line_leaves_a_gap_where_the_dash_is_off`,
  `a_default_style_paints_a_solid_line`, `the_bitmap_context_writes_a_png_of_its_size`).
- `springboard/apps/freeform-desktop/src/freeform.rs`: inspector rows and panel (`ROWS`, `Row`, `Edit`,
  `draw_inspector`, `inspector_hit`, `row_control`), styles kept with the model (`sync_styles`, `Snapshot` with looks),
  `edit`, `export_as_png`, `Bitmap` export, `BAR_INSPECTOR` (14), and host tests (`the_inspector_width_and_fill_set_the_selected_strokes_look`,
  `larger_scales_the_selection_about_its_centre`, `the_inspector_panel_narrows_the_board_and_its_rows_hit_their_segments`,
  `the_export_is_a_png_of_the_board_size_at_the_display_factor`).
- `springboard/apps/freeform-desktop/src/main.rs`: `mod style`, `main_menu` (File > Export as PNG...), `menu_action`.
- `springboard/apps/freeform-desktop/Cargo.toml`, `manifest.toml`: version 1.5.0.
- `springboard/apps/freeform-desktop/CHANGELOG.md`: 1.5.0 entry.
- `docs/parity/wire5-freeform.md`: this note.

## Checks

- `python3 tools/check-names.py`: no hits.
- Hand-worked values: a 10 x 10 square on a 20 x 10 canvas paints ten pixels of row 5 (x 0 to 9). Half opacity
  gives a = round(127.5) = 128, and mix(255, 0, 128) = 127. A 20-pixel line of 2 pt width at scale 1 with dash 6 on
  and 4 off paints x 0 to 6 and 10 to 16, so pixel 8 stays paper. A 10 to 30 stroke about its centre (20) at 1.25
  gives 7.5 and 32.5. The panel is 220 pt, which at 1.5625 is 343.75 px and truncates to 343, so an 800 px board
  gets 457 px of canvas.
