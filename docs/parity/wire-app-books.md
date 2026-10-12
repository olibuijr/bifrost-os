# Wire: Books (find in book, highlights, reading position)

Round: wire the parity models into the native Books app (`springboard/apps/books-desktop/`), version 1.3.0.
Branch `wire/app-books`. Scope: PDF page layout, find in book, highlights and annotations, reading position.

## What is now wired

| Feature (matrix slug, feature) | BifrOSt path | Status after this round |
|---|---|---|
| `appkit-text`, "TextKit: custom layout, text attachments, text lists, find bar" | `springboard/ui/src/textfind.rs` (`find_all`, `Options::whole_word`) through `bifrost_app::controls::textfind` | Partial: whole-word matching in the Books find. The find-bar counter and `TextFinder` state are not used. |
| `pdfkit`, "Selection and find" | `springboard/apps/books-desktop/src/books/mod.rs` (`step_index`, `find_step`, `keep_whole_words`) | Partial: Find Next and Find Previous through the book's hits, and whole-word matching. `pdfselect.rs` (`Selection`, `find`) is not used: Books searches its own reflowed text, not PDF page text objects. |
| `pdfkit`, "Annotations" (highlight) | `books/store.rs` (`Span`, `Entry::highlights`, `toggle_highlight`), `books/draw.rs` (highlight fill) | Partial: highlights are text spans in the reader's own positions, saved per book. `springboard/ui/src/annotation.rs` (`PageAnnotations`, `Subtype::Highlight`) is not used: the reader reflows text and has no page-space bounds to hang an annotation on. |
| Reading position (no matrix row) | `books/store.rs` (`Pos`, `set_pos`), `books/mod.rs` (`remember`, `show_book`) | Already wired before this round. Unchanged. Resume is by block and character offset, so it survives font size changes. |

## How a user reaches it

Open a book in Books (reader view). The menu bar has a new **Find** menu:

- **Find Next** (⌘G) and **Find Previous** (⇧⌘G): step through the matches of the search (the Search panel
  of the reader's bottom bar). They are enabled when the search has hits. They wrap at both ends.
- **Highlight Match** (⇧⌘Y): highlights the text the last find went to. Shown checked when that text is already
  highlighted; choosing it again removes the highlight. Enabled only after a find.
- **Match Whole Words**: limits the search to whole words ("cat" does not match "concatenate"). Checked state
  shows the choice; the session keeps it, and an open search runs again.

Highlights are drawn in the reader's yellow highlight tint under the text, at any font size and after a restart.

## Verify on the test PC (Midget)

Use Midget only (no VM). Build with the fast profile, publish, then OTA until `phase=accepted`:

1. `bifrost pc midget ota`, then `bifrost pc midget state` (Books running at the new version 1.3.0).
2. Put a PDF or EPUB with repeated words in `~/Books` on Midget. Open it in Books.
3. Open the Search panel, type a word that appears on many pages, press Return. Check the **Find** menu:
   Find Next and Find Previous are enabled. Press ⌘G several times and check the page moves to each hit in
   order and wraps.
4. Pick a word that is part of a longer word. Turn on **Match Whole Words** and search again: the hit count drops
   and the page only moves to the whole word.
5. Run Find Next to a match, choose **Highlight Match**. The yellow tint stays on the word when turning pages back
   to it. Change the font size with the Display panel: the highlight still sits on the same word.
6. Close Books, reopen the same book: the highlight is still there and the reading position is restored.
7. Choose **Highlight Match** again on the same match: the highlight goes away.
8. Screenshot with `bifrost pc midget screen /tmp/books-find.png`. Check the menu and tint against
   `docs/golden-gate-sdk-adoption.md` (one tint, no second accent).

Host checks (run by the lead's build, not this round): `cargo test -p bifrost-books` in `springboard/`. The new tests
are in `springboard/apps/books-desktop/src/books/highlight_tests.rs`: step wrap, whole-word filtering, highlight
toggling and save/load (including a highlight line written before its book line, and state files without highlights).

## REA evidence

- `ev_99606441401b2873ddf4c9f74881446c37b21c87d7d30a698cfa867b6ce3e8b7`: PDFAnnotation (official DocC page,
  rendered; annotation types, text markup annotations, creation).
- `ev_f97e9b5a08c3b8b17bd498109a333982d2ff17acdfeb706ab679ee64ac628b88`: PDFSelection (official DocC page,
  rendered; selection of text across a document, pages and strings).
- Also the matrix's existing PDFKit evidence `ev_1e800ce4155bafe135d5af0c8f46309700cd1a9b225b9d6354b17be54c6f57a2`.

Official references read with `python3 -I tools/parity/docc.py pdfkit/pdfannotation` and `pdfkit/pdfselection`.

## Remaining gaps

- **PDF page layout** (`pdfview.rs`, `pdfpage.rs`, PDFView display modes, zoom, thumbnails): not wired. Books reflows
  PDF text into its own page model, so Apple's page geometry, fit modes and page rotation are not shown. Wiring needs
  a fixed-layout page view in the reader (a larger round).
- **Find counter and TextFinder state** ("n of m" in the bar): not drawn. Find Next, Previous and whole words are the
  wired part.
- **Selection** (`pdfselect.rs`): no text selection in the reader, so highlights come from a find match only. Free
  selection with a mouse drag, and the Highlight menu for it, is the next step.
- **Annotation model** (`annotation.rs`): highlights are text spans, not page annotations. Notes, ink and stamps are
  not wired. Ink and stamps need page-space bounds the reflowed reader does not have.
- **Highlights list**: highlights are not listed in the Bookmarks panel yet; they are only drawn and kept.
- **Whole-word option** is session-only (not saved in Settings). Find searches the first 300 hits before filtering,
  so a whole-word search can show fewer than the real count when the book has many partial matches.
- **Matrix rows** are not changed in this branch (`docs/parity-matrix/` is out of scope). After Midget verification,
  the `pdfkit` rows "Selection and find" and "Annotations" can be promoted to Partial with this note.

## Files

- `springboard/apps/books-desktop/src/main.rs` (Find menu, menu actions)
- `springboard/apps/books-desktop/src/books/mod.rs` (find steps, whole words, highlight toggle)
- `springboard/apps/books-desktop/src/books/store.rs` (`Span`, highlight lines `H`, `toggle_highlight`)
- `springboard/apps/books-desktop/src/books/draw.rs` (highlight fill for saved spans)
- `springboard/apps/books-desktop/src/books/highlight_tests.rs` (new tests)
- `springboard/apps/books-desktop/Cargo.toml`, `manifest.toml` (1.3.0), `CHANGELOG.md`
- `docs/parity/wire-app-books.md` (this note)

`springboard/Cargo.lock` has the `bifrost-books` entry at 1.2.5. It is not edited here (outside the app folder); the
lead's build refreshes it.
