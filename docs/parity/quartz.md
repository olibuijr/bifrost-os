# Quartz parity (slug quartz)

Official reference: https://developer.apple.com/documentation/quartz

REA evidence of the rendered framework page: `ev_785e790b371318807d1dd7aa7f8f22ed4ec659d31eb5ce193106fc12c14d19a5`.

Matrix row data: `docs/parity-matrix/data/quartz.json`.

## Team-k refresh (branch parity/team-k-quartz)

Not built or run on this branch; the lead builds and checks it. Verified on Midget: no.

Rows changed:

| Row | Change | Code |
|---|---|---|
| Image decoding for apps | Adds the SDK wrapper note: decode, PNG or JPEG encode | `springboard/sdk/src/graphics_imageio.rs` |
| PDF document model | `os` to `partial`: apps can open a PDF, read page count and sizes, render a page | `springboard/sdk/src/graphics_pdf.rs` (`Document`), `springboard/render/src/cg/pdf/doc.rs` |
| PDF writing and print | Adds PdfWriter and Preview's annotated save through `cg::pdf::writer` | `springboard/sdk/src/graphics_pdf.rs` (`PdfWriter`), `springboard/render/src/cg/pdf/writer.rs` |

The Quartz Composer row stays `gap`. Quartz Composer compositions have no counterpart here, and the row's
deprecation status was not checked against the official pages, so no model is added for it.

## Remaining gaps

- SDK image view and image browser (IKImageView, IKImageBrowserView): image viewing is inside the Photos shell app only.
- Parameterised image filters (CIFilter) and a filter browser.
- Scanner support and a camera device browser.
- Quick Look panel drawing in the shell and an embeddable preview view.
- PDF outline, selection and text access for apps.
- PDF annotation and form widget reading in the engine.
- Print-to-PDF filter pipeline (QuartzFilter).
