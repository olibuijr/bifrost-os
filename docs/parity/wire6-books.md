# Wire 6: Books (display modes, selection, PDF highlight annotations)

Round: wire the PDF display, selection and annotation models into the native Books app
(`springboard/apps/books-desktop/`), version 1.6.0. Branch `wire6/books`, from `main` (release 0.5.43). Not built here:
the lead builds once (`cargo test -p bifrost-books` in `springboard/`, then the compliance and snapshot tests).

## What is now wired

| Feature (matrix slug, feature) | Model | BifrOSt path | Status after this round |
|---|---|---|---|
| `pdfkit`, "PDFView (display and navigation)" | `springboard/ui/src/pdfview.rs` (`DisplayMode`) | `books/mod.rs` (`display`, `two_pages`, `set_two_pages`, `metrics`, `turn`), `books/draw.rs` (`draw_reader`, `draw_page`), `main.rs` (View menu) | Partial: View > Single Page and View > Two Pages. Two Pages paginates the text to half the width and draws two pages side by side. Continuous scrolling is not wired. |
| `pdfkit`, "Selection and find" | `springboard/ui/src/pdfselect.rs` (`DocumentText`, `Selection`) | `books/mod.rs` (`page_text`, `document_text`, `select_page`, `has_selection`, `selection_text`), `books/draw.rs` (selection tint), `main.rs` (Find > Select Page, Find > Copy Selection) | Partial: whole-page selection (both pages of a spread) and copy to the shared clipboard. No drag selection and no per-word selection. Find keeps its own path. |
| `pdfkit`, "Annotations" | `springboard/ui/src/pdfannot.rs` (`Annotation`, `Subtype::Highlight`, `PageAnnotations`) | `books/mod.rs` (`pdf_highlights`), `books/draw.rs` (`draw_page` annotation pass) | Partial: for PDF books, each saved highlight is a highlight annotation with its note as the contents. The annotations are drawn under the text. Other formats keep the span drawing. Notes are still stored in the book's state file. |

The Pages panel, the Notes panel, Find, Highlight Match, Add Note and the Contents panel are unchanged from wire 5.

## How a user reaches it

- **View > Single Page** and **View > Two Pages** (the View menu merges into the shell's View menu). The current choice
  is checked. Both are enabled while a book is open. Two Pages keeps the page you are on: the spread starts with it.
  Turning a page (tap an edge or swipe) moves two pages in a spread. The choice is kept for the session, not across a
  restart.
- **Find > Select Page** selects the text on the screen (both pages of a spread). A selected line is tinted.
  **Find > Copy Selection** copies the selected text to the shared clipboard as plain text. Turning a page or changing
  the text size clears the selection.
- **PDF highlights**: choose Find > Highlight Match or Find > Add Note on a find match in a PDF book. The highlight is
  drawn from its annotation. Its note is the annotation's contents; it is shown in the Notes panel as before.

## Verify on the test PC (Midget)

Use Midget only (no VM). Build with the fast profile, publish, then `bifrost pc midget ota` until `phase=accepted`.

1. `bifrost pc midget state`: Books is running at 1.6.0.
2. Open a PDF with several pages. Open the View menu: Single Page is checked.
3. Choose **View > Two Pages**. Two pages sit side by side, each narrower than before, with a gap between them. The
   Single Page item is now unchecked and Two Pages is checked.
4. Tap the right edge of the page (or swipe left): the spread moves two pages on, with no slide animation.
5. Choose **Find > Select Page**. Both pages of the spread are tinted line by line. Choose **Find > Copy Selection**,
   then paste into a text field (for example the Search field of Books): the text of both pages appears, one page after
   the other.
6. Turn a page: the tint goes away. Choose **View > Single Page**: the reader shows one page at the same place.
7. Search for a word (Search panel, Return). Choose **Find > Find Next** until the word is on the page, then
   **Find > Highlight Match**. The word is highlighted yellow. Choose **Find > Add Note**, type "test note", press Return.
8. Close Books and reopen the same PDF. The highlight is still drawn and the note is still in the Notes panel.
9. Compare with `bifrost-hig`: one tint (the Select Page tint is the same blue as the other tints), the page backgrounds
   follow the reader theme (White, Sepia, Dark). Screenshot with `bifrost pc midget screen PATH`.

Host tests (lead's build): `cargo test -p bifrost-books` in `springboard/`. New tests in
`books/tests.rs`: `two_pages_spread_turns_by_two_and_selects_both_pages` (spread pagination, a turn moves two
pages and does not slide, Select Page takes both pages in order, a turn clears the selection, return to one page keeps
the place), `select_page_copies_the_text_on_one_page` (the page text of page one, starts with "Chapter One"),
`pdf_highlights_are_annotations_with_their_notes` (one highlight annotation with its note as contents, left edge at
x = 0, and no annotations for an EPUB book).

## REA evidence

- `ev_f66ea39d22b044af9e7ef6476ff1c88338bf5f0c41edde4478613d82293e7b1e`: PDFView (official DocC page, rendered; document,
  current page, visible pages, display configuration, navigation).
- `ev_581bff98c6ba42ae71648e64d05915c534a608b274a9607723c9d194c4fba288`: PDFSelection (official DocC page, rendered;
  pages, string, bounds for a page, selections by line).
- `ev_41e41f20b9109806aa5d3c4cf49f58c1fcf13c02be6a79c968b62d0666417f23`: PDFAnnotation (from wire 5; creation with
  bounds and type, `contents`, `isHighlighted`, text markup).

Official references read with `python3 -I tools/parity/docc.py pdfkit/pdfview`, `pdfkit/pdfselection` and
`pdfkit/pdfannotation`.

## Remaining gaps

- **Continuous scrolling** (View > Continuous): not wired. `pdfview.rs` already has `SinglePageContinuous` and
  `TwoUpContinuous` frames (`PdfView::page_frames`). Books needs a scroll offset over stacked reflowed pages before a menu
  item can be added; the item is left out rather than shown disabled.
- **Two Pages turns by spread, not by a slide**: no animation for the spread change.
- **Selection is whole-page only**: no drag or word selection, no selection by touch. `pdfselect.rs` supports ranges, so a
  touch model is the next step. Find matches are still their own highlight.
- **Annotations are not PDF page annotations**: they are not written back into the PDF, and no ink, stamp or shape
  annotation exists. Highlights stay in the book's state file.
- **Annotation bounds are reflow bounds**: a highlight's rectangle is measured on the reflowed page, not the PDF's own
  page space, so it is not the original glyph box.
- **Thumbnails are text** (wire 5 gap, unchanged), **display of an annotation's note on the page** (no marker, wire 5
  gap, unchanged).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| `pdfkit` | PDFView (display and navigation) | `springboard/apps/books-desktop/src/books/mod.rs` (`set_two_pages`, `turn`), `springboard/apps/books-desktop/src/main.rs` (View menu) |
| `pdfkit` | Selection and find | `springboard/apps/books-desktop/src/books/mod.rs` (`select_page`, `selection_text`), `springboard/apps/books-desktop/src/books/draw.rs` (selection tint) |
| `pdfkit` | Annotations | `springboard/apps/books-desktop/src/books/mod.rs` (`pdf_highlights`), `springboard/apps/books-desktop/src/books/draw.rs` (`draw_page`) |

## Files

- `springboard/apps/books-desktop/src/books/mod.rs` (`PAGE_GUTTER_PT`, `page_text`, `document_text`, `Reader::selection`,
  `Books::display`, `metrics`, `repaginate`, `goto_page`, `turn`, `two_pages`, `set_two_pages`, `select_page`,
  `has_selection`, `selection_text`, `pdf_highlights`, imports of `pdfannot`, `pdfselect`, `pdfview::DisplayMode`)
- `springboard/apps/books-desktop/src/books/draw.rs` (`draw_reader` two-page branch, `draw_page` annotation and selection
  passes, ribbon position on a spread)
- `springboard/apps/books-desktop/src/main.rs` (`MENU_SINGLE_PAGE`, `MENU_TWO_PAGES`, `MENU_SELECT_PAGE`,
  `MENU_COPY_SELECTION`, Find items, View menu, `menu_action` arms)
- `springboard/apps/books-desktop/src/books/tests.rs` (three new tests)
- `springboard/apps/books-desktop/Cargo.toml`, `manifest.toml` (1.6.0), `CHANGELOG.md`
- `docs/parity/wire6-books.md` (this note)

`springboard/Cargo.lock` has the `bifrost-books` entry at 1.5.0. It is not edited here (outside the app folder); the lead's
build refreshes it.
