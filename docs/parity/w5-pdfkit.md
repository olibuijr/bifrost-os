# Wave 5: PDFKit gap rows (view layout and form widgets)

Two pure Rust models in `springboard/ui/src` close two of the PDFKit gap rows. Neither is wired into an app or the
shell yet, and neither has been built or run on the VM or Midget. Both are std-only and have host unit tests.

## Built

### `springboard/ui/src/pdfview.rs` (`pub mod pdfview`)

The layout model behind a document view: display mode (single page, single page continuous, two-up, two-up
continuous), fixed scale factor with limits, fit-to-width autoscaling, page frames in document space, scroll
position, current page, visible pages, go-to-page, next page and previous page. A view draws the frames; the model
does not draw.

Official API mirrored:
- `PDFView`: `currentPage`, `visiblePages`, `displayMode`, `scaleFactor`, `autoScales`, `minScaleFactor` and
  `maxScaleFactor` (as `scale_limits`), `goToNextPage(_:)` and the previous-page counterpart.
- `PDFDisplayMode`: `singlePage`, `singlePageContinuous`, `twoUp`, `twoUpContinuous`.

Choices that are ours, not Apple's: the default scale limits (0.1 to 16), the default page gap (10 points), the
row grouping in two-up modes (pairs from the first page, no cover-page rule), and the fit-to-width rule (the
widest row fits the viewport width). Navigation history (`currentDestination`, back and forward) and page overlay
or markup mode are not modelled.

### `springboard/ui/src/pdfform.rs` (`pub mod pdfform`)

The model for form widgets: widget subtype (button, choice, text, signature), button control (check box, radio
button, push button), flag decoding (read-only, toggle-to-off, multiline, password, comb, combo, editable choice),
maximum text length in characters, list options and export values, the value and default value, reset, and a
radio group selection rule (`select_radio`). Flag bits are the PDF form field flag bits. Parsing the AcroForm
field tree and writing a value back to the file are not modelled.

Official API mirrored:
- `PDFAnnotation` widget properties: `widgetFieldType`, `widgetStringValue`, `widgetDefaultStringValue`,
  `fieldName`, `isReadOnly`, `isMultiline`, `isPasswordField`, `maximumLength`, `hasComb`, `widgetControlType`,
  `buttonWidgetState`, `allowsToggleToOff`, `choices`, `isListChoice`, `values`.
- `PDFAnnotationWidgetSubtype`: `button`, `choice`, `signature` (plus `text`, which the official text widget
  section covers).
- `PDFWidgetCellState` (`on`, `off`, `mixed`) and `PDFWidgetControlType`.

## REA evidence

- `ev_420eb744af4df9b80c1593cae33e240624ef36cad05a5ded2a6b2547e60cae47`: PDFView page, rendered at
  https://developer.apple.com/documentation/pdfkit/pdfview
- `ev_8de6933c544ef3ebbadba7a17a0248d4e7cba8cd430544abc2c482ac1b4e1322`: PDFAnnotation page, rendered at
  https://developer.apple.com/documentation/pdfkit/pdfannotation

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/pdfkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| pdfkit | PDFView (display and navigation) | partial | springboard/ui/src/pdfview.rs |
| pdfkit | Widgets and form fields | partial | springboard/ui/src/pdfform.rs |

## Tests

- `pdfview.rs`: continuous rows stack with the page gap and centre horizontally; two-up shows only the current row;
  scale clamps and fit-to-width; current page and visible pages follow the scroll; navigation clamps; bad page
  sizes are refused.
- `pdfform.rs`: button flags pick the control; text flags decode; maximum length counts characters; read-only
  fields refuse changes and reset; list and editable choices; export value fallback; radio group holds one
  selection; toggle-to-off rule; check box and push button.

Not run: no cargo build and no test run in this wave, per the wave rules.

## Still open in PDFKit

- Write operations (save with edits, permissions, password unlock): still a gap.
- Page overlay and markup mode: still a gap.
- PDFView delegate events and navigation history: not modelled.
