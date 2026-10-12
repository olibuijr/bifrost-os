# Changelog

## 1.6.2

- The procedural app icon is built only for its tests; the shipped icon is `icon.png`. The state report (`describe`) is built only for its tests too: serving it over the control socket needs a JSON dependency this app does not have yet.

## 1.6.1

- Removed the unused constructor, the default folder helper, the unused forget and needs_frame helpers; the host passes its library folders.

## 1.6.0

- View menu: Single Page shows one page of a book at a time, as before. Two Pages shows a spread: two pages side by
  side, with the text set to the narrower column. A turn moves a whole spread. Continuous scrolling is not in this
  release.
- Select Page (Find menu) selects the text on the screen: both pages of a spread. Copy Selection puts the selected text
  on the shared clipboard. A turn to another page clears the selection.
- PDF books: the highlights you make are kept as highlight annotations, each with its note, so a note travels with its
  highlight. Other formats draw their highlights as before.

## 1.5.0

- Find menu: Add Note writes a note on the text the last find went to. The text is highlighted if it was not, and the
  Notes panel opens with the note field ready. Press Return to save the note, or Escape to stop editing. An empty note
  removes the note and keeps the highlight. Notes are kept with the book, across restarts.
- The reader's Contents panel has two new tabs. Pages lists every page with the first line of its text, so you can
  jump to a page by its opening words. The current page is bold. Notes lists the notes in reading order with their page
  numbers; tap one to go to it.

## 1.4.0

- Find menu: Read Aloud starts reading the chapter from the page you are on to its end. The sentence being read is
  shaded and the book turns its pages to follow it. Pause Reading and Resume Reading continue it, and Stop Reading
  ends it. No voice is installed on this PC yet, so nothing is spoken: each sentence is held for the time it takes to
  read it aloud, and a note on the page says so.
- Find menu: Look Up and Translate act on the text the last find went to. Look Up names the word and the language the
  sentence is in; there is no dictionary on this PC yet, so it gives no definition. Translate says why a translation
  cannot run: no language packs are installed yet. A tap on the page clears the note.
- The library's main area uses the window's content colour (white in Light, dark grey in Dark) instead of black.

## 1.3.0

- Find menu: Find Next (⌘G) and Find Previous (⇧⌘G) step through the matches of the book's search in order and wrap at
  the ends. Match Whole Words limits the search to whole words, so "cat" skips "concatenate". The choice lasts for the
  session.
- Highlight Match (⇧⌘Y) marks the text the last find went to. Highlights stay on the page at any font size and across
  restarts, and choosing it again removes the highlight.
- Highlights are saved with the book's reading position in the Books state file, one line per highlight.
- Edit > Copy Quote (⇧⌘C) copies the sentence around the match the last find went to, with its citation (the book's
  title, author and chapter), as plain text on the shared clipboard. It is enabled while the match is highlighted.
- Desktop search finds books by their title and author, and picking a book there opens it in Books.

## 1.2.5

- File › Print… (⌘P) prints the open PDF book through `bifrost_app::print` (CUPS `lp`); the item merges into the shell's File menu.

## 1.2.4 — 2026-10-11

- Library search: the toolbar has a Golden Gate search field at the trailing end (`widgets::search_field`), so the
  root screen publishes a text field like Books for Mac. It narrows the shelf by title and author, and the collection
  counts follow it; a clear button empties it and Esc or Return leave the field. Tapping a cover or a sidebar row
  takes the focus away.
- Reading themes (White, Sepia, Dark) draw from named `READER_*` tokens in `render/src/ui.rs`, so no literal colours
  are left in the app and the reader keeps its own palette whatever the system appearance is.

## 1.2.3 — 2026-10-11

- Compliance pass (app compliance gate): worker results reach the app loop through a shared inbox that the main
  queue fills, so the channels and the `next_wake` worker feed are gone; the library probe and the book open
  share `Inbox` and `finished`.
- Colours: translucent overlays (cover spine, highlight, author line, icon rules) derive from named tokens with
  `fade`; the icon tile is the system orange token.
- Layout: the corner radii come from `prefs_layout::m` (tile and card); empty states, the loading box, the chapter
  line and the Text size label are placed from the window and the panel size; the theme swatch row and cover
  column width are named layout constants.

## 1.2.2 — 2026-10-10

- Snow Leopard: opening a book and the library cover probes run on the dispatch pool and wake the app loop on
  completion; the 150 ms polling timer is gone.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate look (full-size window, system 0.4.84): a source-list sidebar with the Library collections
  (Reading Now, All, Books, PDFs) and a unified toolbar with a sidebar button and the collection title.
- The reader keeps a unified toolbar like Books for Mac: Contents and Bookmark at the leading edge, the book title
  centred, Search, Display (Aa) and Close Book at the trailing edge; the iPad back chevron and text bar are gone.
- Contents, bookmarks, search and display open in a pane at the leading edge instead of covering the page.
- The library follows Light and Dark; toolbar buttons are published as controls. Esc dismisses the search field.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 26 padding and gap values moved to the nearest standard step (at most 2 pt).
- 9 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Books for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Route physical text, Backspace and Return keys to existing field editors.
- Scan local document folders, open file URLs with escaped names, and flush reading state on close.
