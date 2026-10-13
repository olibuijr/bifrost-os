# Terminal polish (round 75)

Slug `terminal-polish`. Terminal lives in `springboard/src/shell/terminal/` (shell code, not an SDK app).

## Reference evidence

- Shipped Terminal 2.15 (`com.apple.Terminal`) `Info.plist`, read directly from
  `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Terminal.app/Contents/Info.plist`:
  URL schemes `telnet`, `ssh`; document types for Terminal session, settings and shell script; `x-man-page`.
- `Base.lproj/MainMenu.nib` menu labels (string table, read directly): Close Split Pane, Jump to Previous/Next Mark,
  Mark as Bookmark, Insert Bookmark, Clear to Previous Mark, Copy Plain Text, Copy Special, Copy Without Background Color,
  Enter Full Screen, Find Previous, Hide Find Bar, Edit Background Color, Default Font Size, Cycle Through Windows,
  Automatically Mark Prompt Lines, Allow Mouse Reporting, Hide Alternate Screen, Clear Scrollback, Fill Screen,
  Built-in with Same Command.
- REA (`open_binary`, `inspect_plist`, `decode_interface_builder`) could not be used: the extracted bundle has no
  `Contents/MacOS/Terminal` program, and the nib is not a supported binary format for REA. The nib labels above
  came from a plain string read. Re-measure with REA once the extract includes the program file.

## Rows (per slice)

| Feature | State before | State after | BifrOSt path |
|---|---|---|---|
| Tabs (new, close, switch, Cmd+1..9, next/previous) | Done | Unchanged | `terminal/window.rs`, `terminal/tabs.rs`, `terminal/input.rs` |
| Find in scrollback (Cmd+F, next/previous, count) | Done | Unchanged | `terminal/find.rs`, `terminal/menus.rs` |
| Copy and paste (Cmd+C only copies a selection, Cmd+V pastes) | Done | Unchanged | `terminal/input.rs`, `termtouch.rs` |
| Copy Plain Text / Copy Special | Copy only | Copy only (see below) | `terminal/menus.rs` |
| URL open | Detection only (`profile_table::detect_urls`, not wired) | Wired: More menu "Open link in selection" opens the first web link in the selected text in the browser | `terminal/links.rs`, `terminal/menus.rs` (arm 15) |
| Profiles (colours, text size, cursor) | Two sources: `terminal/profile_table.rs` (Terminal.app table, unused) and `termcore::palette::THEMES` (in use) | Unchanged; merge still open | `terminal/profiles.rs`, `terminal/profile_table.rs` |
| Secure Keyboard Entry | Missing | Missing (see below) | none |
| Split panes | Missing (the shell's split view excludes Terminal, `splitview.rs`) | Missing | none |
| Window groups, New Command | Missing | Missing | none |

## What is done

- **Open link in selection.** Pure helper `links::first_link(text)` returns the first link in reading order using the
  existing `profile_table::detect_urls`. The More menu has a new last row, index 15, `Open link in selection`. It reads
  the selection, opens the first `http`/`https` link through `Shell::desktop_open_url`, and reports `Opened link`,
  `Could not open link: ...`, or `No link in selection`. Tests: `links::tests::first_link_is_taken_from_selected_rows`,
  `tests::open_link_reports_when_the_selection_has_no_link`.
- A row was appended at the end on purpose, so the existing indices of the More menu (and the tests that assert them)
  stay put.

## Not done, and why

- **Tap on a link in the grid.** Touch input has no Cmd-click. A tap already brings the keyboard back, and a long press
  selects. Opening links from the selection avoids a new gesture. Still open: a link affordance (underline on hover
  equivalent) and a long-press menu item.
- **Profiles merge.** `profile_table::PROFILES` (Terminal.app Basic, Pro, Homebrew ...) is still dead code under
  `#[allow(dead_code)]`. The live path uses `termcore::palette::THEMES` in another crate. Merging needs the palette
  crate to take the table, so it is left for a follow-up. Cursor style and blink exist in `Profile` but are not drawn.
- **Secure Keyboard Entry.** No model yet. On a touch shell, there is no other process that can read the keyboard
  input stream the way Terminal guards it, so the menu item needs a design decision before it is added.
- **Split panes, window groups, New Command.** Not started. The splitview code excludes Terminal on purpose
  (`splitview.rs`, test at line 679). Window groups need the window manager's grouping model. New Command needs a
  command palette UI.
- Copy Plain Text and Copy Special are not separate: copy already writes plain text only.

## Midget checks still needed

- More menu: the new row fits, scrolls, and reads "Open link in selection" at phone and desktop sizes.
- Select a line containing `https://` text, open the More menu, tap the row, and confirm the browser opens the page.
- Select text with no link and confirm the message `No link in selection` shows in the panel.
