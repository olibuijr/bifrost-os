# Theme 21: PDFPage rendering and drawing

## What was built

`springboard/ui/src/pdfpage.rs` (`bifrost_ui::pdfpage`, pure model, std only) adds the page geometry that a PDF page
needs before anything is drawn:

- `DisplayBox` (media, crop, bleed, trim, art) with `PageBoxes::bounds` and `PageBoxes::set_bounds`. Unset boxes fall
  back along a chain (crop to media; bleed, trim and art to crop), so setting a box moves the boxes that still follow it.
- Rotation: `normalize_rotation` and `PdfPage::set_rotation` store 0, 90, 180 or 270 clockwise. Negative and larger
  multiples wrap; other angles are refused.
- `PdfPage::displayed_size` (width and height swap for a quarter turn), `PdfPage::transform_for` (page space onto a
  display box, rotated, origin at the box corner, as an `AffineTransform`), `PdfPage::fit_transform` (aspect-fit,
  centred, for drawing into a target rectangle), and `PdfPage::thumbnail_size` and `fit_scale` (aspect-fit size for
  thumbnails).
- `PdfPage::from_image_size`: a page whose media box is an image's size in points, at the origin.

The module does not draw, render text or handle annotations. The renderer applies the returned transforms.

## Apple APIs mirrored (official documentation)

- `PDFPage` (https://developer.apple.com/documentation/pdfkit/pdfpage): `bounds(for:)`, `setBounds(_:for:)`,
  `rotation`, `transform(for:)`, `transformContext(for:)` (deprecated; mirrored as the transform), `init(image:)`.
- `PDFDisplayBox` (https://developer.apple.com/documentation/pdfkit/pdfdisplaybox): `mediaBox`, `cropBox`, `bleedBox`,
  `trimBox`, `artBox`, with the default values the page documents (crop defaults to media; bleed, trim and art default
  to crop).
- `thumbnail(of:for:)` (https://developer.apple.com/documentation/pdfkit/pdfpage/thumbnail(of:for:)): the size part
  only. The image is not produced here.
- `init(image:options:)` (https://developer.apple.com/documentation/pdfkit/pdfpage/init(image:options:)): the size
  part only. `ImageInitializationOption` is not mirrored.

## REA evidence

- `ev_6e3586b2c4edb0da9b94d6dc389970fa14ed70cb49fd2dc65c4f15287c7f52c8`: rendered `PDFPage` landing page
  (`tools/parity/rea-page.sh`).

## Matrix rows closed

| Framework slug | Feature (as in data file) | New status | BifrOSt path |
|---|---|---|---|
| pdfkit | PDFPage: rendering and drawing | partial | springboard/ui/src/pdfpage.rs |
| pdfkit | PDFThumbnailView and page thumbnails | partial | springboard/ui/src/pdfpage.rs (thumbnail size only; no view, no rendering) |
| pdfkit | PDFPage: page image and PDF creation | partial | springboard/ui/src/pdfpage.rs (`from_image_size`; no image option, no PDF data output) |

## Not closed

- Annotations, selections and find, PDFView display, write options, widgets, overlay and markup mode (other rows of
  this theme) are untouched.
- Rows of other frameworks in this theme (PhotoKit, Quartz, Application Services, VisionKit, Quick Look, Media
  Intelligence) are not addressed.
- Drawing into a context, the page `thumbnail(of:for:)` image, and `PDFDocument` loading are left to the renderer.
