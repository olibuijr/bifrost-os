# Changelog

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
