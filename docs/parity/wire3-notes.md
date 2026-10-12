# Wire 3: Notes sketches (apps/notes-desktop)

Scope: drawing and sketch attachments in a note, on the drawing model (`ui/src/drawing.rs`, the PencilKit and PaperKit
stroke and ink models), and the checklist symbol rendering modes (`ui/src/symbol_layers.rs`). Owned folder:
`springboard/apps/notes-desktop/`. Version 1.12.0 (manifest.toml, Cargo.toml, CHANGELOG.md).

## What is wired

| Feature | Where | How a user reaches it |
|---|---|---|
| Sketch pad: pen strokes on a sheet over the window, saved as a picture in the note | `apps/notes-desktop/src/notes/sketch.rs` (`Pad`, `dabs`, `stroke_dabs`, `render`, `encode`, `save`, `load`), `src/notes/mod.rs` (`open_sketch`, `sketch_touch`, `draw_sketch`, `finish_sketch`, `insert_image`) | Edit a note, More menu > Insert Sketch. Drag to draw. Header: Cancel (left), Undo (middle), Done (right). Done puts the picture at the caret. |
| Drawing saved as data beside the picture | `sketch.rs` `save` writes `<id>-<n>.sketch` (the `Drawing::to_text` form) next to `<id>-<n>.png`; `load` reads it back | Not visible in the UI yet; the file moves, copies, recovers and erases with the note (`folders.rs` `attachments`) |
| Numbering shared by pictures and drawings | `folders.rs` `next_attachment` now takes one past the highest number in use (was: count of files plus one) | Automatic |
| Note background in the content column | `src/notes/chrome.rs` `draw_background`, `src/notes/mod.rs` row swipe fills: `CONTENT_BG` instead of `DETAIL_BG` | Automatic (white in Light, dark grey in Dark) |

The pad reuses the shell's photo sheet pattern (a sheet with its own header, a touch handler and a draw pass), and the
note's own image path (`attach::import` rules, inline display through `load_images`).

## How to verify on Midget

1. `bifrost pc midget update --restart`, then `bifrost pc midget open-app is.olibuijr.notes.desktop`.
2. Open or create a note, type `Plan`, then More > Insert Sketch. The pad covers the window with a header.
3. Drag a line across the pad. Tap Undo: the line goes. Draw again, then tap Done.
4. Expected: the pad closes and the note now reads `Plan` followed by an inline picture of the line (white paper,
   black stroke, a 16 px margin). The note folder holds `<id>-1.png` and `<id>-1.sketch`.
5. Open More > Insert Sketch, tap Cancel: nothing is inserted and no file is written.
6. Screenshot with `bifrost pc midget screen`, and compare the pad and the picture in Light and Dark
   (`bifrost pc midget appearance light|dark`).

Host tests (not run in this round; the lead builds once): `sketch.rs` (`dabs_overlap_and_include_both_ends`,
`pad_draws_undoes_and_refuses_blank`, `a_picture_is_cropped_to_the_strokes_with_a_margin`,
`a_huge_drawing_is_scaled_to_the_longest_side_limit`, `the_picture_is_a_png_that_the_attachment_rules_accept`,
`save_writes_the_picture_and_its_drawing_and_load_reads_the_strokes_back`), `mod.rs`
(`insert_sketch_saves_a_picture_and_its_drawing_beside_the_note`), `folders.rs` (the existing
`moving_a_note_carries_its_attachments`, which still expects `100-2.png` after `100-1.png`).
Expected values were worked out by hand: a 10 px line with a 2 px brush gives 11 dabs (one per px); a 40 px line
gives a 72 x 32 picture (40 + 2 x 16 wide, 0 + 2 x 16 tall); a 4000 px line is scaled to a 1600 px side.

## Reference evidence

- Official pages, via `tools/parity/docc.py` (the `pencilkit` collection, `pencilkit/pkcanvasview`):
  PencilKit is "capture touch and Apple Pencil input as a drawing, and display that content from your app"; the
  canvas stores its captured input in a `PKDrawing`.
- REA rendered pages (`tools/parity/rea-page.sh`):
  - `ev_47df90ac6b2e4271286b7b164b7d2d06fe5c345a3a0a79a1b382b92967fd00c9`: PKCanvasView (canvas view, drawing stored as
    PKDrawing, tool picker observer, `allows Finger Drawing`).
  - `ev_0e965c9b6d26c9af6d64f41f8500607d01aa98812b4c840ed84e6d7adba1d787`: PKDrawing (stores the user-drawn content,
    saved with the app's data, can make a new drawing from saved data, can generate an image).
- `docc.py` for `paperkit` returned the collection page (PaperMarkupViewController and the markup views); it was used for
  the "Freehand drawing tools" row only.

## Not wired, and why

- **Checklist symbol rendering modes (`ui/src/symbol_layers.rs`).** The checklist boxes are drawn by the shell's text
  area (`springboard/src/shell/textarea.rs`, `md::Block::Check`), which is outside this app's folder. The layer model
  has no draw hook for a glyph, so `RenderingMode`, `HIERARCHY` and `palette_slot` are not used. Needs a shell change.
- **Colours, marker, pencil and eraser in the sketch pad.** Only black pen lines. `INK_COLOR` is a constant; the
  `InkKind` enum has the kinds but the pad only creates `Pen`.
- **Reopening a sketch to edit its lines.** `sketch::load` reads the `.sketch` file, but no UI opens a sketch for
  editing. A tap on a sketch picture does nothing.
- **Stylus pressure, tilt and time.** The pad stores `StrokePoint::at(x, y)`: no force, azimuth or altitude, since the
  app's touch events carry only position. The `pressure` row stays partial.
- **Lasso, stroke slicing, Bezier import, tool picker, handwriting.** Not used. `ui/src/lasso.rs`, `Stroke::slice`,
  `Drawing::from_path` and the tool palette are not reachable from the app.
- **Undo for drawings in the note.** The pad has its own Undo (strokes). The note's text Undo is separate.
- **Markup elements (shapes, text boxes, links, loupe).** `ui/src/markup.rs` is not used; the pad is freehand only.
- Carried over from wire 2 and unchanged: the multi-line text view, flight and address underlines, and tapping a
  detected item.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| pencilkit | Canvas view capturing pen and finger input | springboard/apps/notes-desktop/src/notes/mod.rs (`sketch_touch`, mouse or touch; no pen data) |
| pencilkit | Drawing model (PKDrawing) and save as data | springboard/apps/notes-desktop/src/notes/sketch.rs (`save`, `load`; `<id>-<n>.sketch`) |
| pencilkit | Stroke and point model (PKStroke, PKStrokePath, PKStrokePoint) | springboard/apps/notes-desktop/src/notes/sketch.rs (`Pad`, `stroke_dabs`) |
| pencilkit | Ink type (PKInk: type, colour, width) | springboard/apps/notes-desktop/src/notes/sketch.rs (`INK_COLOR`, pen ink only) |
| pencilkit | Undo for drawings | springboard/apps/notes-desktop/src/notes/sketch.rs (`Pad::undo`, reached through the pad's Undo) |
| pencilkit | Rendering to an image | springboard/apps/notes-desktop/src/notes/sketch.rs (`render`, `encode`, PNG) |
| paperkit | Freehand drawing tools | springboard/apps/notes-desktop/src/notes/sketch.rs (pen only, in the note's sketch pad) |
| symbols | Symbol weights, scales and rendering modes | not wired (checklist glyphs are drawn by springboard/src/shell/textarea.rs, outside this app) |

## Parity status

The rows above stay `partial` until the sketch pad is checked on Midget: a row is `parity` only once it works there
(`bifrost pc parity`, skill `bifrost-parity-matrix`). No matrix data was edited in this round.
