# Viewer polish (slug: viewer-polish)

Scope: the Viewer app (`springboard/apps/preview-desktop`, bundle `is.olibuijr.preview.desktop`), version 0.3.0.

## Reference evidence

- Preview.app bundle, Info.plist (document types, handler ranks, related item types): `~/Projects/macos-ref/extract/ref27/System/Applications/Preview.app/Contents/Info.plist`.
  Read by listing its keys with grep (REA `inspect_plist` not run in this round). Only key names were used to shape the document types; no strings were copied.
- Preview.app resources: `Contents/Resources/{Base.lproj,en.lproj}` (menu and label tables). Not yet measured with REA in this round, so menu wording is from the existing app.
- ServicesMenu.strings under `en.lproj` is present; not used by this round.

## Rows

| Area | BifrOSt path | Status |
|---|---|---|
| Lazy thumbnails (sidebar) | `apps/preview-desktop/src/document.rs` (`Document::thumb`, `thumb_box`), `src/canvas.rs` (`rows`, `draw_sidebar`), `src/model.rs` (`fit_box`) | Done in code. A page's thumbnail is drawn the first time the sidebar shows it and kept per page. Rows are laid out from the thumbnail box, which comes from the page size, so no thumbnail is drawn to lay out the list. Needs a Midget check on a long PDF. |
| Fast open | `document.rs` (`pdf_pages`) | Partly done. Opening reads the file, parses it and reads the page sizes; it no longer draws any thumbnail. The file read and parse are still synchronous on the UI thread: the PDF document is `Rc`-based and not `Send`, so a background open needs a worker that hands over the parsed document. Still a model. |
| Export JPEG | `document.rs` (`export_jpeg`), `canvas.rs` (`Action::ExportJpeg`), `main.rs` (File menu) | Done in code. Uses the shell's JPEG encoder (`shell::camera::jpeg::encode_pixels`, quality 90), with rotation, Adjust Color and markup burned in. Writes `<name>-markup.jpg`. |
| Export PNG | `document.rs` (`export_png`, shared `export_picture`) | Refactored onto the same picture path as JPEG. Behaviour unchanged. |
| Rotate | `model.rs` (`rotate`, `View::turn`), `canvas.rs` | Existed. The View menu item was enabled from an unbound value; it is now computed from the page. |
| Markup: text, shapes, signature, saved into the PDF | none | Gap. Markup is still a sidecar file keyed by path (`model::to_sidecar`) and burned in on export. Writing annotations into a PDF needs a PDF writer, which the PDF engine does not have. |
| Export as PDF | none | Gap. Needs the same PDF writer. |
| Reorder and delete pages | none | Gap. Page indices are used for markup, navigation and the PDF page list; reordering needs a page-order map that the sidecar also stores. Not started. |
| Image size | none | Gap. Adjust Color has exposure, saturation and contrast only. |
| Sidebar contents (outline) | none | Gap. The PDF outline is not read yet. |

## Still needs checking on Midget

- Thumbnails appear as the sidebar scrolls, on a PDF of at least 200 pages, and the scroll stays smooth.
- Export JPEG writes a file that opens, with the markup and the rotation in it.
- The View menu's Rotate items are enabled on an image and on a PDF page, and disabled with no document.

## Shared-file edits

None. All edits are inside `springboard/apps/preview-desktop`, plus `springboard/Cargo.lock` (the `bifrost-viewer` version line only).
