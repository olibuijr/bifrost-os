# Files polish (parity slice files-polish)

Scope: apps/files daily-driver behaviour. Round 75 (Files 1.26.0) finishes the tab strip and Empty Trash, and records
the state of the rest of the slice.

## Reference evidence

- Finder bundle: `~/Projects/macos-ref/extract/ref27/System/Library/CoreServices/Finder.app` (Info.plist and
  `en.lproj` strings, `Base.lproj/FinderView.nib`). Searched for tab-bar and "New Tab" wording in the en.lproj strings
  and the Base.lproj nib names: none found in this extract, so the strip's show-with-two-tabs rule is a design choice
  to check on Midget, not a measured behaviour. No REA evidence ids were recorded in this round.

## Done in this round

- Tab strip (Files 1.26.0). Model: `apps/files/src/tabs.rs`, `TabStrip<T>` (unchanged API). Helpers: `cells` (tab cell
  layout, at most 180 pt each), `hit` (select or close box, the close box 16 pt wide 6 pt in from the cell's right edge).
  Drawing: `Files::paint_tab_strip` under the toolbar, 30 pt tall, drawn only with two or more tabs. The open tab is
  drawn raised on the content colour; the others are secondary-coloured. Clicks: `Files::tab_hit` in `window.rs`
  `click`, then `select_tab` or `close_tab_at` (closing a tab other than the open one parks the open tab's state
  first). Content moves below the strip through `Files::body_top` and `Files::rows_top` (replacing the fixed 60/94/110
  offsets in the list, icon, column and gallery views, the options panel and the scroll clamp).
  Tests: `tabs::tests` (cells, hit), `main` test `the_tab_strip_shows_with_two_tabs_and_moves_the_content_under_it`.
  - Keys (unchanged): ⌘T new tab, ⌘W close open tab, Ctrl+Tab and Ctrl+Shift+Tab step, Shift+T Tags.
  - Needs Midget check: strip appearance and the close box hit area, the content offset in icon, list, column and
    gallery views, the label width when many tabs are open, and dark mode colours.
- Empty Trash (Files 1.26.0). Job kind `JobKind::Empty` in `ops.rs`, runner `empty_now` (deletes each item through the
  coordinated write path, then prunes the origins file), action `Files::empty_trash` (lists the bin, skipping
  `.origins`; a notice when there is nothing to delete). Entry points: ⇧⌘⌫ (`keys.rs`) and the background context menu
  while in the Trash (`MENU_EMPTY_TRASH`, `menus.rs`). Tests: `emptying_the_trash_deletes_every_item_for_good`,
  `empty_trash_says_so_when_the_bin_has_nothing_to_delete`.
  - Not done: a confirmation sheet (Finder asks first) and undo. Both need a decision before Midget use.
  - Needs Midget check: the job's progress line and the notice.

## Already in code (not changed this round)

- Get Info: `apps/files/src/info.rs` (`get_info`, `paint_info`), with tests for permissions, ISO and EXIF rows.
- Tags: `info.rs` (`open_tags`, `toggle_tag`, `paint_tag_panel`), colour dots, the Tags sidebar section (`/.tagged`).
- Go to Folder: `apps/files/src/go.rs` (`open_goto`, `go_chord`, path completion), with tests.
- Open With: `apps/files/src/openwith.rs` (`open_with_panel`, the default app marked), with tests.
- Drag and drop inside Files: `main.rs` (press, drag past the slop, drop targets in folders and the sidebar, spring-loaded
  folders in `go.rs`, Option copies).

## Not done

- Drag and drop between Files and other windows or apps. The SDK has the protocol (`sdk/src/dnd.rs`: `begin`,
  `register_types`, `Event::Dest`), but a file payload needs a read grant (`Payload::File { grant, .. }`), and the shell
  hands grants over only for a document it opened. Files has no grant source for a plain folder listing yet, and no
  `App::drag` handler. Needs a shell-side grant path before this can be built.
- Confirmation before Empty Trash and undo of it (see above).
- Per-tab selection and view mode; tab drag-reorder; a New Tab control in the strip (⌘T only).
- Sidebar customisation, rename, duplicate and trash rows are unchanged from earlier rounds and were not re-measured
  against the reference.

## Matrix

No `docs/parity-matrix/os` entry was closed by this round: the rows that mention Finder or tabs are framework rows, not
Files behaviour, and were left alone.
