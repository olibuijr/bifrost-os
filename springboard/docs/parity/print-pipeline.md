# Parity: print pipeline (round 75)

Slice: printing everywhere. One SDK print operation that apps use, the system print sheet, Save as PDF through the
shared `cg::pdf` writer, queue submission through the CUPS client. Per-app private print code is deleted.

## Evidence

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `PrintCore.framework` (ApplicationServices) and the
  extracted `ref27/System/Applications/Utilities/Print Center.app`. The Print Center `Info.plist` was not measured with
  REA this round (`inspect_plist` needs an open target), so behaviour decisions rest on the repo's print model and the
  CUPS/IPP wire. Re-measure with REA before closing the menu-level rows.
- Code evidence: `ui/src/print.rs`, `ui/src/cups.rs`, `render/src/cg/pdf/writer.rs`, `sdk/src/print_panel.rs`,
  `src/shell/print_sheet.rs`.

## Rows and paths

| Row | API | BifrOSt path | State |
|---|---|---|---|
| Print operation (`NativePrintOperation`) | `PrintOperation` (file, pages, text sources) | `ui/src/print.rs`, SDK `bifrost_app::print` | Done for host; submit path needs Midget check |
| Print info (`NativePrintInfo`) | `PrintInfo` paper, orientation, copies, page range | `ui/src/print.rs` | Done; margins stay in the document (not an IPP attribute) |
| Printable area (`imageableBounds`) | `PrintInfo::imageable` | `ui/src/print.rs` | Done |
| Print panel (printer, copies, pages, paper, orientation, preview) | shell sheet | `src/shell/print_sheet.rs` | Shell sheet has all rows; app-to-shell routing still open (apps call the SDK directly) |
| Plug-in print panels | `PlugIns::job_options` | `sdk/src/print_panel.rs` | Done; panels are not drawn by the shell yet |
| Save as PDF | `Destination::Pdf` | `render/src/cg/pdf/writer.rs` | Done; writer is host-tested |
| Queue submission | Print-Job, default printer lookup | `ui/src/cups.rs` `print_document_with`, `default_printer` | Done; replaces the `lp` shell-out |

## What changed

- `render/src/cg/pdf/writer.rs` (new): one PDF writer. Standard-font pages with WinAnsi text, RGB images as FlateDecode
  streams, valid xref. Used by TextEdit, the shell sheet, and the SDK text source.
- `ui/src/cups.rs`: `print_document_with` sends job-template attributes (copies, media, orientation-requested,
  page-ranges). `print_document` keeps its signature.
- `ui/src/print.rs`: `Source` (file, pages, text). `run` submits through the CUPS client; `Save as PDF` writes the same
  bytes. `PrintError::NoPrinter` added. `available()` is a local TCP probe of the scheduler port. Plain text prints
  in Helvetica with the standard AFM widths, word-wrapped to the page.
- `sdk/src/print_panel.rs`: `lp_options` becomes `job_options` (key/value pairs). Reserved keys are the attributes
  `PrintInfo` writes.
- `apps/textedit-desktop`: private PDF object writer, WinAnsi encoder and temp file deleted; the layout-to-text step
  stays and feeds `PrintOperation::pages`. 1.1.1.
- `src/shell/print_sheet.rs`: private `write_pdf` deleted; `pdf_pages` builds one image page per slice and `Job::run`
  goes through `PrintOperation`. Temp file deleted.
- `apps/preview-desktop` (Viewer): File > Print (Cmd+P). Prints the file as opened; markup is not drawn. 0.4.0.
- `apps/notes-desktop`: Note > Print (Cmd+P), plain text, on its own thread. 1.15.0.
- `apps/mail-desktop`: Message > Print (Cmd+P), headers and decoded body as text, on its own thread. 0.4.0.
- `apps/books-desktop` already used `PrintOperation::new(path)`; unchanged.
- Browser: still no print (Servo 0.7 has no print API).

## Still needs Midget checking or follow-up

- A real CUPS submission (IPP attribute acceptance for `media`, `orientation-requested`, `page-ranges` on the target
  queue) and the Save as PDF file in Documents.
- The shell sheet is not reachable from apps: an app Print command prints through the SDK's default printer, not the
  sheet with its printer and paper rows. Routing an app request to the sheet needs a shell request path.
- Viewer prints the original file, so markup is missing from printouts.
- TextEdit and Notes/Mail text layouts use different metrics (TextEdit uses the window's font measure, the text source
  uses Helvetica AFM widths). Long words run past the right edge (no hyphenation).
- Print panel plug-ins are still not drawn by any dialog.
