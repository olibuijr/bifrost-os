# Wave 3: PDFKit parity (selection, find, annotations)

Branch `parity/w3-pdfkit`, started from `parity/integration` (waves 1 and 2). Pure Rust, std only, no unsafe, no new
crates. Models only: nothing here draws, and nothing is wired into the Books app, the PDF reader or the SDK yet.

## What was built

### `springboard/ui/src/pdfselect.rs` (bifrost-ui)

Text selection and find over a document's page text, as a pure model.

- `DocumentText`: pages of text in reading order. `page_count`, `page`, `number_of_characters`.
- `Selection`: ranges `(page, character range)` kept in page order. `between(doc, from, to)`, `add` (merges ranges that
  overlap or touch, so noncontiguous selections are supported), `pages`, `string`, `selections_by_line`.
- `find(doc, query, options)`: one selection per match, in document order. It reuses `textfind::find_all` and its
  `Options` (case-insensitive by default, whole-word optional), so matching agrees with the text fields. Byte offsets
  from that model are converted to character indices.

Apple names mirrored (official reference, `PDFKit`): `PDFSelection` (`pages`, `string`, `selectionsByLine()`, `add(_:)`),
`PDFDocument selection(from:at:to:at:with:)` (approximated by `Selection::between`), `PDFDocument` find
(`FoundSelection` results approximated by `find`). Not mirrored: `extend(atStart:)` and `extend(atEnd:)`,
`extendForLineBoundaries()`, `selectionForWord`/`selectionForLine`, `attributedString`, `characterBounds`, the
`color` and drawing methods, and the `PDFDocument` object itself (there is no app-facing document yet).

Choice: `string` joins separate ranges with `\n`. Apple's `string` on a noncontiguous selection is not specified in the
official page, so this is a BifrOSt decision, recorded here.

### `springboard/ui/src/pdfannot.rs` (bifrost-ui)

Annotations as a pure model.

- `Subtype`: text, free text, ink, highlight, underline, strikeout, squiggly, square, circle, line, stamp, link, widget.
- `Annotation`: bounds in page space (validated: finite, non-negative size), display and print flags, contents, user
  name, border width (validated), highlighted state, `contains(x, y)` (the rectangle rule of `Rect::contains`).
- `PageAnnotations`: the annotations of one page in drawing order. `add` (on top), `remove(index)`, `get`,
  `annotations`, `annotation_at(x, y)` (topmost displayed annotation under the point; hidden annotations are not hit),
  `printed()` (annotations with the print flag).

Apple names mirrored (official reference, `PDFAnnotation`): `PDFAnnotation init(bounds:forType:withProperties:)`
(`Annotation::new`, without the properties dictionary), `PDFAnnotationSubtype` (`Subtype`), `bounds`, `contents`,
`userName`, `shouldDisplay`, `shouldPrint`, `border` (as a width only), `isHighlighted`, `PDFPage addAnnotation(_:)`,
`removeAnnotation(_:)`, `annotation(at:)` (`annotation_at`). Not mirrored: the `annotationKeyValues` dictionary and
`setValue(_:forAnnotationKey:)` family, `PDFBorder` dash patterns, `PDFAction`/`PDFDestination`, `draw(with:in:)`,
`interiorColor`, `font`/`fontColor`, the line `startPoint`, and `hasAppearanceStream`.

## Official API evidence

- Official pages read through `tools/parity/docc.py`: `pdfkit/pdfannotation`, `pdfkit/pdfselection`; the PDFKit
  landing page was rendered through `tools/parity/rea-page.sh`.
- REA evidence (PDFKit landing page, rendered): `ev_b2de1bf20defa86ee03b429f52ab83278ccb9ba09c6356b5af3d550eb98f7991`.
- Wave 2 reference evidence for the PDFKit matrix: `ev_1e800ce4155bafe135d5af0c8f46309700cd1a9b225b9d6354b17be54c6f57a2`.

## Matrix rows closed

These rows should move from `gap` to `partial` in the central matrix. The lead updates the matrix; no matrix file
was edited here.

| framework slug | feature (as written in `docs/parity-matrix/data/pdfkit.json`) | new status | BifrOSt path |
|---|---|---|---|
| pdfkit | Selection and find | partial | springboard/ui/src/pdfselect.rs |
| pdfkit | Annotations | partial | springboard/ui/src/pdfannot.rs |

Rows still open for PDFKit: PDFView (display and navigation), Widgets and form fields, Write operations, and Page
overlay and markup mode.

## Tests

No builds were run in this wave (repo rule). Each module has a `#[cfg(test)] mod tests` with hand-worked values:

- `pdfselect.rs`: 9 tests (between within a page and across pages, refused positions, find order and case, whole-word
  find, character indices for non-ASCII text, line splitting, add merging and gaps).
- `pdfannot.rs`: 9 tests (bounds validation, defaults, border validation, add and remove, topmost hit test, hidden
  annotations, half-open edges, printed filter, contents and author).

The owner name guard `python3 tools/check-names.py` reports no hits for the new files.

## Not yet done

- Nothing is wired into the Books reader, the SDK or the shell. The models are host-tested only, not run on the VM or Midget.
- The reader does not yet fill `DocumentText` or `PageAnnotations` from a file (no AcroForm or `/Annots` parsing).
