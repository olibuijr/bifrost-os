# Wire 5: Books (pages panel, notes on highlights)

Round: wire the PDF display and annotation models into the native Books app (`springboard/apps/books-desktop/`),
version 1.5.0. Branch `wire5/books`, from `main` (release 0.5.42). Not built here: the lead builds once
(`cargo test -p bifrost-books` in `springboard/`, then the compliance and snapshot tests).

## What is now wired

| Feature (matrix slug, feature) | BifrOSt path | Status after this round |
|---|---|---|
| `pdfkit`, "PDFThumbnailView and page thumbnails" | `books/mod.rs` (`Panel::Pages`, `panel_rows`, `activate` Row), `books/draw.rs` (`draw_rows` Pages arm) | Partial: a Pages tab lists every page with the first line of its text as a text thumbnail. The current page is bold; a tap goes to the page. No picture thumbnails. |
| `quartz`, "PDF display view (PDFView, PDFThumbnailView)" | same as above | Partial: page list navigation only. No fixed-layout page view and no display modes. |
| `pdfkit`, "PDFView (display and navigation)" | `books/mod.rs` (`Panel::Pages`, `goto_page`) | Partial: page navigation from a list of all pages (the current page is shown). `springboard/ui/src/pdfview.rs` (`PdfView`, `DisplayMode`) is not used: the reader reflows text and has no page-space frames. |
| `pdfkit`, "Annotations" (note) | `books/store.rs` (`Entry::notes`, `set_note`, `note_of`, `N` lines), `books/mod.rs` (`add_note`, `save_note`, `notes_list`, `Panel::Notes`, `NoteField`), `books/draw.rs` (Notes tab and note field) | Partial: a note is written on a highlight and saved per book. `springboard/ui/src/pdfannot.rs` (`Annotation`, `PageAnnotations`) is not used: its bounds are page-space rectangles and the reader has none. |

The existing Find, Find Next, Find Previous, Match Whole Words, Highlight Match and the Contents panel (the outline of
the book, from the PDF outline) were already wired (`wire-app-books.md`); this round does not change them.

## How a user reaches it

Open a book in Books and tap the **Contents** button in the reader bar. The panel now has four tabs:

- **Contents** and **Bookmarks**: as before.
- **Pages**: one row per page. The row shows the first line of text on the page, and the page number. The current page
  is bold. Tap a row to go to that page and close the panel.
- **Notes**: the notes written on highlights, in reading order, with their page numbers. A note field sits above the
  list. Tap a note's row to go to its page and close the panel.

To write a note: search the book (Search panel, then Find Next), then choose **Find > Add Note** (no shortcut). The
match is highlighted if it was not, the Notes panel opens and the note field takes the typing. Type, then press Return
to save. Escape stops the edit. An empty note removes the note and keeps the highlight. Choosing Add Note on the same
match again opens its note for editing.

Notes are kept in the book's state file (`N` lines, alongside the `H` highlight lines), so they survive a restart.

## Verify on the test PC (Midget)

Use Midget only (no VM). Build with the fast profile, publish, then `bifrost pc midget ota` until `phase=accepted`.

1. `bifrost pc midget state`: Books is running at 1.5.0.
2. Open a PDF or EPUB with several chapters. Tap **Contents** in the reader bar. The tabs read Contents, Bookmarks,
   Pages and Notes.
3. Choose **Pages**. Each row shows a page number and the first line of its text. The current page is bold. Tap a row
   two or three pages on: the book turns to that page and the panel closes.
4. Search for a word (Search panel, Return). Choose **Find > Find Next** until the word is on the page. Choose
   **Find > Add Note**: the Notes panel opens with a blinking caret in the note field. Type "test note" and press Return.
   The note appears as a row with its page number.
5. Open **Contents**, choose **Notes**, and tap the note's row: the book turns to its page and the panel closes.
6. Close Books and reopen the same book. Open the Notes tab: the note is still listed, and the highlight is still drawn.
7. Choose **Find > Add Note** on the same match, clear the text (select with the keyboard, delete) and press Return.
   The note row goes away; the highlight stays.
8. Check the panel against `bifrost-hig`: one tint, the note field and tabs use the reader's theme colours in White,
   Sepia and Dark. Screenshot with `bifrost pc midget screen PATH`.

Host tests (lead's build): `cargo test -p bifrost-books` in `springboard/`. New tests:
`highlight_tests.rs::notes_are_trimmed_ordered_and_saved_with_their_highlights` (tab and line-break cleaning, reading
order, save and reload, empty note removal, zero-length span) and `tests.rs::add_note_opens_the_notes_panel_and_saves_what_is_typed`
(Add Note opens the panel and highlights the match, typing and Return save the note, the row jumps to the page),
`tests.rs::pages_row_jumps_to_its_page` (one row per page, a row jumps to its page).

## REA evidence

- `ev_0f9f79c0c85b81b60846fad2ad21c41e7ca4d7a604eb4e9bf5ae141cfccb0170`: PDFView (official DocC page, rendered;
  current page, visible pages, navigation, display configuration, page overlay and markup mode).
- `ev_41e41f20b9109806aa5d3c4cf49f58c1fcf13c02be6a79c968b62d0666417f23`: PDFAnnotation (official DocC page,
  rendered; creation with bounds and type, `contents`, `userName`, `isHighlighted`, `shouldDisplay`, text markup).
- `ev_419fb4806d4ab7d6b0229f76a3e9408c0fd48f1305167e239e326a8c8392a18c`: PDFThumbnailView (official DocC page,
  rendered; `pdfView`, `thumbnailSize`, `maximumNumberOfColumns`, `allowsDragging`, `selectedPages`).

Official references read with `python3 -I tools/parity/docc.py pdfkit/pdfview`, `pdfkit/pdfannotation` and
`pdfkit/pdfthumbnailview`.

## Remaining gaps

- **Display modes** (single page, two pages, continuous scroll): not wired. Books reflows text into one page size, so a
  two-page spread needs a half-width layout and a second page drawn beside the first. `pdfview.rs` (`DisplayMode`,
  `PdfView::page_frames`) is the model to use once the reader has page-space frames.
- **Thumbnails are text**: no page is rasterised. A picture thumbnail needs the page drawn into a small surface, which
  the reader cannot do for reflowed text. `pdfpage.rs` (`thumbnail_size`, `fit_scale`) is ready for the size maths.
- **Notes are not drawn on the page**: a note has no marker in the text. Only the Notes list shows it. A small marker
  at the highlight is the next step.
- **Annotation model** (`pdfannot.rs`): not used. Highlights and notes are text spans in the reader's own positions.
  Ink, stamps and shapes need page-space bounds that the reflowed reader does not have.
- **Selection** (`pdfselect.rs`): still no free text selection, so a note can only go on a find match.
- **Note editing** is through Add Note only. Tapping a note row goes to its page; it does not open the note for editing.
- **Matrix rows** are not changed in this branch (`docs/parity-matrix/` is out of scope). After Midget verification,
  the rows listed above can be promoted with this note.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| `pdfkit` | PDFView (display and navigation) | `springboard/apps/books-desktop/src/books/mod.rs` (`Panel::Pages`, `goto_page`) |
| `pdfkit` | PDFThumbnailView and page thumbnails | `springboard/apps/books-desktop/src/books/draw.rs` (`draw_rows`, Pages arm) |
| `pdfkit` | Annotations | `springboard/apps/books-desktop/src/books/store.rs` (`Entry::notes`, `set_note`), `books/mod.rs` (`add_note`, `Panel::Notes`) |
| `quartz` | PDF display view (PDFView, PDFThumbnailView) | `springboard/apps/books-desktop/src/books/mod.rs` (`Panel::Pages`) |

## Files

- `springboard/apps/books-desktop/src/books/store.rs` (`Entry::notes`, `N` lines, `note_of`, `set_note`)
- `springboard/apps/books-desktop/src/books/mod.rs` (`Panel::Pages`, `Panel::Notes`, `Target::NoteField`, `Reader::note`,
  `Reader::note_target`, `add_note`, `save_note`, `notes_list`, `note_field_rect`, panel input and rows)
- `springboard/apps/books-desktop/src/books/draw.rs` (four tabs, note field, Pages and Notes rows)
- `springboard/apps/books-desktop/src/main.rs` (Find > Add Note, `MENU_ADD_NOTE`)
- `springboard/apps/books-desktop/src/books/highlight_tests.rs`, `springboard/apps/books-desktop/src/books/tests.rs` (new tests)
- `springboard/apps/books-desktop/Cargo.toml`, `manifest.toml` (1.5.0), `CHANGELOG.md`
- `docs/parity/wire5-books.md` (this note)

`springboard/Cargo.lock` has the `bifrost-books` entry at 1.4.0. It is not edited here (outside the app folder); the
lead's build refreshes it.
