# Connect: Books to the system services

Round: connect the native Books app (`springboard/apps/books-desktop/`, version 1.3.0, not bumped) to the shell's
services, so its library and quotes reach the rest of the desktop. Branch `connect/books`, built on
`parity/integration`. Not built or run yet: the lead builds once.

## What is connected

| Service | Matrix slug, feature | BifrOSt path (app) | What it does |
|---|---|---|---|
| `search-index` (Core Search items, the desktop search panel) | `core-spotlight`, "Searchable items and attribute sets" | `src/books/search_index.rs`, `src/main.rs` (`tick`, `continue_activity`) | Each book on the shelf is indexed by its title, its author (as a keyword) and its kind (PDF or EPUB). Picking a book in the search panel opens it in Books. |
| `pasteboard` (typed items on the general board) | `application-services`, "Pasteboard (NSPasteboard)"; `core-transferable`, "Copy and paste of typed items" | `src/books/quote.rs`, `src/main.rs` (Edit menu, `menu_action`), `src/books/mod.rs` (`quotation`) | Edit > Copy Quote writes the sentence around the match the last find went to, with its citation (title, author, chapter), as one plain-text item on the shared clipboard. |

Not connected: `now-playing`. Books reads no text aloud and plays no audio, so it has nothing to publish as a Now
Playing session. Nothing was added for it.

Service calls are event driven. The search index sends only the books that are new, changed or removed since the last
frame (`search_index::SearchIndex::sync` compares the shelf with what it last sent); a frame with no change sends
nothing. The copy is a single pasteboard write on the menu action.

## How a user sees it

- **Search panel:** type a word from a book's title or author in the desktop search panel. The book appears under
  Documents. Choosing it opens the book in Books. Items appear once the shelf has its titles (the covers are read in
  the background, so a new book can take a moment to show its real title).
- **Edit > Copy Quote (⇧⌘C):** open a book, search for a word with the Search panel, press ⌘G to go to a match, and
  choose Edit > Copy Quote. The item is enabled while the match is highlighted on the page. Paste it in any text field:
  the sentence in curly quotes, then `— Title, Author, Chapter` on the next line.

## How to verify on the test PC (Midget)

Use Midget only (no VM). Build with the fast profile, publish, then `bifrost pc midget ota` until `phase=accepted`.

1. Put two books in `~/Books` on Midget with different authors. Open Books once so the covers and authors are read.
2. Open the desktop search panel and type the author's name. The book must be listed. Choose it: Books opens that book.
3. Rename the author in the file's metadata (or add a second book with the same title word) and check the search panel
   follows the change without a restart.
4. Open a book, search for a word in it with the Search panel, press ⌘G until the match is on the page. Choose
   Edit > Copy Quote. Paste into a text field (for example the search field of another app): the sentence and the
   citation must be there, in that order, with the chapter name when the book has a contents list.
5. Clear the match (close the search panel) and check Edit > Copy Quote is disabled.
6. Screenshot with `bifrost pc midget screen PATH` and check the Edit menu against `bifrost-hig` (one tint, no second
   accent).

Host tests (lead's build, not run here): `cargo test -p bifrost-books connect_tests` from `springboard/`. They check the
index messages (one send per change, a delete when a book leaves, nothing when nothing changed), the search-result
hand-off domain, the sentence and citation text (including Icelandic characters and an over-long sentence), the
pasteboard write body (general board, plain text), and the quotation on a live find match.

## Gaps

- **No free text selection in the reader.** A quotation comes only from the match the last find went to, so a
  reader cannot copy an arbitrary passage yet. Needs a selection model in the reader (a larger round).
- **Page numbers are not cited.** The reader reflows text, so its pages are not the book's printed pages. The citation
  is title, author and chapter only.
- **Chapter comes from the table of contents.** A book without a contents list gets no chapter part.
- **Index holds titles and authors only.** Highlights and text are not indexed; the shell keeps at most 1000 items per
  app, and long identifiers (over 128 bytes, long paths) are refused by the shell and so not indexed.
- **Renamed files are new items.** The item's identifier is the book's path, so a book renamed on disk is indexed
  again under the new path, and the old item is removed when the shelf rescans and the old path has left.
- **Search index items are not reported back.** The app gets no event when an item is removed by the shell.
- **Pasteboard reads are not used.** Books only writes quotes; it does not read the general board (no paste into the
  reader).
- **Shell tests pending.** Nothing has been built or run on a device in this round.

## Files

- `springboard/apps/books-desktop/src/books/search_index.rs` (new): index sync, the search-result hand-off.
- `springboard/apps/books-desktop/src/books/quote.rs` (new): sentence, chapter, citation, quotation and the pasteboard write.
- `springboard/apps/books-desktop/src/books/connect_tests.rs` (new): unit tests for the glue.
- `springboard/apps/books-desktop/src/books/mod.rs` (module lines, `quotation`, `index_entries`).
- `springboard/apps/books-desktop/src/main.rs` (Edit menu, `menu_action`, `tick`, `continue_activity`).
- `springboard/apps/books-desktop/CHANGELOG.md` (1.3.0 entry extended, version not bumped).
- `docs/parity/connect-books.md` (this note).
