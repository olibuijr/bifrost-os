# Parity: Viewer prints the markup (round 75)

Slice: Viewer's File > Print prints the edited document, not the file on disk. Viewer opens PDFs off the UI thread with
lazy thumbnails.

Reference evidence: `~/Projects/macos-ref/extract/ref27/System/Applications/Preview.app/Contents/Info.plist` is
extracted, but was not measured with REA in this round. Behaviour decisions rest on the repo's own Viewer model
(`docs/parity/viewer-finish.md`) and the print pipeline (`docs/parity/print-pipeline.md`).

## Rows

| Row | API | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|---|
| Print the document with its annotations (Print panel prints the marked up document) | PDF print of annotated pages | `apps/preview-desktop/src/printjob.rs` (`pdf_job`, `prepare`, `print_file`), `src/pdfmarks.rs` (`write_edited`, shared with Export PDF), `src/main.rs` (`Viewer::print`, `pump`) | A PDF's page order, deletions, turns and markup are written to `<data>/print/<name>.pdf` on the PDF worker through `cg::pdf::writer::save`, then sent through `PrintOperation`. The original is not changed. A picture prints its marked up page (Adjust Color, turns, markup) as a PNG. The status line says Printing, then Sent to the printer or the error. | A real printer must print the marked up copy (annotations drawn by the printer's PDF reader). |
| Open PDFs off the UI thread, lazy thumbnails | `Document` open and page drawing | `apps/preview-desktop/src/pdfjob.rs`, `src/document.rs` (`thumb`, `picture`, `request`) | Already in place (Viewer 0.3.0 and 0.4.0): the worker parses the file and reports sizes and outline; thumbnails and pages are asked of the worker when shown and kept after. Nothing changed in this round. | Check that a long PDF opens without a stall on Midget. |

## What changed

- `src/pdfmarks.rs`: the save body moved into `write_edited(pdf, source, target, order, turns, marks)`. `save_job`
  (Export PDF) calls it, so print and save share one edit path. No second copy.
- `src/pdfjob.rs`: `Reply::Printed(String)` carries the status line back to the window.
- `src/printjob.rs` (new): `scratch_file`, `message` (PrintError to status line), `print_file` (sends a file through
  `PrintOperation`), `pdf_job` and its `prepare` step.
- `src/main.rs`: `Action::Print` calls `Viewer::print`. The `PrintOperation` import and the status mapping moved to
  `printjob`, and `pump` handles `Reply::Printed`.
- Version 0.5.1 in `manifest.toml`, `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md`.

## Behaviour notes

- A PDF print runs on the worker, so the window is not blocked by the file write or the CUPS submit. Page draws queue
  behind a print on the same worker until it is sent.
- The print copy is overwritten on each print. The job reads it at submit time.
- Rotated pages keep their turn in the printout, as in Export PDF.

## Not done

- The Print panel still submits through the SDK's default printer, not the shell sheet (open in print-pipeline.md).
- The print copy stays in the data folder after printing; it is not cleaned up.

## Matrix

Closes the Viewer note in `docs/parity/print-pipeline.md` ("Viewer prints the original file, so markup is missing from
printouts"). The matrix rows under `docs/parity-matrix/os/` were not edited.
