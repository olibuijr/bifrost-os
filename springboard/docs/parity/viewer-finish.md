# Viewer finish (slug: viewer-finish)

Scope: the Viewer app (`springboard/apps/preview-desktop`, bundle `is.olibuijr.preview.desktop`), version 0.4.0, on top of
the PDF engine's writer added in `springboard/render/src/cg/pdf/writer.rs`. Builds on `viewer-polish` (0.3.0).

Nothing here is compiled or run in this round (the round's rules: code and commits only; the lead builds). Every row
below still needs a build, the unit tests and a Midget check before it counts as parity.

## Reference evidence

- Only `ServicesMenu.strings` of the Viewer's reference bundle has been extracted so far
  (`~/Projects/macos-ref/extract/ref27/System/Applications/Preview.app/Contents/Resources/en.lproj/`). The menu tables
  (Base.lproj nibs) were not there, so the menu wording is the existing app's and the design's, not measured from the
  reference. REA was not run in this round for that reason. The PDF object model comes from the engine's own code and the
  PDF specification's annotation dictionary keys, not from the reference.

## Rows

| Area | API (reference name) | BifrOSt path | Status |
|---|---|---|---|
| Open off the UI thread | PDFDocument init(url:) | `apps/preview-desktop/src/pdfjob.rs` (`spawn`, `Pdf::open`, `deliver`), `main.rs` (`open_path`, `pump`), `document.rs` (`from_pdf`) | Done in code. One worker thread per PDF owns the parsed file (the engine's objects are reference counted, so they cannot leave the thread). The open result and every drawn page come back through `DispatchQueue::main()`, and `App::tick` takes them. Needs a Midget check on a 200 page file: the window stays live while it opens. |
| Lazy page thumbnails | PDFThumbnailView, PDFPage.thumbnail(of:for:) | `document.rs` (`thumb`, `thumb_box`, `request`, `store`), `canvas.rs` (`draw_pages`) | Done in code. A thumbnail is asked of the worker the first time its row shows, kept, and at most `PDF_KEPT` pictures are kept besides the thumbnails. Thumbnails are drawn unrotated (see limits). |
| Page picture | PDFPage draw(with:to:), transformContext(for:) | `document.rs` (`picture`), `canvas.rs` (`draw`: white paper until the picture arrives) | Done in code, on the worker. |
| Markup into the PDF: text note | PDFAnnotation (text) | `pdfmarks.rs` (`annotation`, Note arm), `cg::pdf::writer` | Done in code. A note becomes a /Text annotation with its text as UTF-16, and a 14 point blue marker as its appearance. |
| Markup into the PDF: highlight | PDFAnnotation (highlight) | `pdfmarks.rs` (Highlight arm) | Done in code. /Highlight with QuadPoints; the appearance uses a multiply blend at 45 per cent. |
| Markup into the PDF: shapes | PDFAnnotation (square) | `pdfmarks.rs` (Rect and Rounded arms) | Done in code. Rectangles are /Square with a 2 point border. Rounded rectangles are written as squares (no corner curves in the appearance). |
| Markup into the PDF: signature (freehand stroke) | PDFAnnotation (ink) | `model.rs` (`MarkKind::Ink`, `Mark::ink`, sidecar encoding), `main.rs` (`Gesture::Ink`, `stroke`), `canvas.rs` (`draw_mark`), `pdfmarks.rs` (Ink arm) | Done in code. A new Sign tool records the stroke's points; it is drawn in the viewer, in PNG and JPEG exports, and written as an /Ink annotation with an /InkList. |
| Save markup into a PDF | PDFDocument write operations | `cg::pdf::writer::save` (incremental update), `pdfmarks::save_job`, `main.rs` (`export_pdf`) | Done in code, as Export PDF (File menu) writing `<name>-markup.pdf` beside the original. The original is not changed. Every save is read back by the engine and every written object must load as written, or the save fails. |
| Reorder and delete pages | PDFDocument insert/remove/move page | `document.rs` (`order`, `move_page`, `delete_page`), `main.rs` (`act`: Move Page Up/Down, Delete Page; Pages menu), `writer.rs` (`save`, flat page tree only) | Done in code. The shown order and deletions are saved by Export PDF. A nested page tree cannot be reordered (the save says so). Deleted pages' markup is not written. |
| Rotate and save | PDFPage rotation | `document.rs` (`turns`, `set_turn`, `turn_of`), `main.rs` (`rotate`), `pdfmarks.rs` (`edits`: /Rotate), `writer.rs` | Done in code. Rotate keeps its turn with the page (it was per view), so Export PDF writes /Rotate as the page's own rotation plus the turns. Rotation of pictures is kept for the session and in the export. |
| PDF export | PDFDocument dataRepresentation | `main.rs` (`export_pdf`), `pdfmarks.rs` (`save_job`) | Done in code (the same save as above). |
| Image resize | (no reference name) | `export.rs` (`finish`), `canvas.rs` (`Action::ExportAt25/50/100/200`), `main.rs` (File menu) | Done in code. Export at 25, 50, 100 and 200 per cent. A picture is resized with the engine's bilinear resize; a PDF page is drawn at the scaled box, so text stays sharp. |
| Outline sidebar | PDFOutline: child(at:), label, destination | `document.rs` (`outline`), `pdfjob.rs` (`Outline`, `summary`), `canvas.rs` (`outline_rows`, `draw_outline`, `fit_text`), `main.rs` (`touch_down` Outline arm, View menu) | Done in code. The sidebar's Outline button (or View > Outline) lists the engine's outline with its depth; a click goes to the page the entry names, when that page is still shown. |

## Limits

- Thumbnails are drawn unrotated; a turned page shows its thumbnail the old way up (the row size follows the page).
- Rounded rectangles are saved as square annotations.
- Export PDF writes a new file; it does not rewrite the original in place. An encrypted PDF cannot be saved this way.
- A PDF page tree that is nested cannot be reordered or have pages deleted; rotation and markup still save.
- The annotation appearance of a highlight uses the multiply blend, which some readers do not apply; the colour still shows.
- Page reorder, delete and turns are kept for the session and saved by Export PDF only; the sidecar keeps only the markup.

## Still needs checking on Midget

- A PDF of 200 or more pages opens without freezing the window; the sidebar fills as it scrolls.
- Export PDF writes a file that opens in another reader, with the highlights, rectangles, notes and the signature drawn, and with the page order and rotation as shown.
- Delete Page and Move Page change the sidebar, the canvas and the saved file together.
- The Outline button lists the document's outline and a click lands on the right page.
- Export at 200 per cent of a PDF page is sharp, and Export at 25 per cent writes a smaller picture.

## Tests added (not run here)

- `render/src/cg/pdf/writer.rs`: rotation and an annotation read back; deleting and reordering a flat tree; refused orders;
  an unchanged file for no edits; the page-point mapping; the startxref read.
- `apps/preview-desktop/src/model.rs`: ink round trip through the sidecar and burn-in.
- `apps/preview-desktop/src/pdfmarks.rs`: highlight annotation, turns and deletions, strokes, text encoding.
- `apps/preview-desktop/src/export.rs`: export names, scaled and turned output, unchanged output.
- `apps/preview-desktop/src/document.rs`: page moves and deletes, turns follow the source page, a picture has no worker.
- `apps/preview-desktop/src/canvas.rs`: every toolbar button is a menu action, export scales, the Sign tool.

## Matrix rows (PDFKit, `docs/parity-matrix/data/pdfkit.json`)

Not edited (the lead closes rows). Rows this work advances, pending a build and a Midget check:

- Outline (table of contents): the Viewer's outline sidebar.
- Annotations: text, highlight, square and ink annotations written into the PDF.
- Write operations (save): Export PDF with page order, deletion and rotation.
- PDFPage: rendering and drawing: pages drawn on the worker thread for the Viewer.

## Shared-file edits

- `springboard/render/src/cg/pdf/mod.rs`: one line (`pub mod writer;`).
- `springboard/Cargo.lock`: the `bifrost-viewer` version line (0.4.0).
- No edits to `sdk/src/lib.rs`, `ui/src/lib.rs` or `view/mod.rs`.
