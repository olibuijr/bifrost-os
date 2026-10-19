# Connect: Files to the system services (springboard/apps/files)

Version 1.17.2 (not bumped this round; the CHANGELOG top entry is extended). Branch `connect/files`. Status: written and
unit-tested by reading only. Not built, not run. The lead's integration build runs the tests; Midget is the device test.

## What is connected

| Service (SDK client) | Matrix slug (docs/parity-matrix/data) | What Files does now | Code |
|---|---|---|---|
| `quick-look` (`bifrost_app::quick_look`) | application-services / quick look and preview rows (Partial) | Space shows the shell preview panel for the selection, starting at the focused row. Left and Right step it; the focused row follows the item shown (`index` events). Space again, Esc or the close button hides it (`closed` event). Preview menu item does the same. Without the panel (an older shell, or an error reply) the in-app preview opens as before. | `springboard/apps/files/src/system.rs` (`quick_look`, `follow_preview`, `preview_event`, `service_answer`) and `main.rs` (`service_event`, Space routing), `menus.rs` |
| `pasteboard` (`bifrost_app::pasteboard_types`) | application-services "Pasteboard (NSPasteboard)" (Partial); core-transferable "Copy and paste of typed items" (Partial) | Copy and Cut write the selection to the general board as typed items: one item per file with a file URL representation (`public.file-url`) and its name as plain text (`public.utf8-plain-text`). Paste reads the board: file URLs become the paste sources. If the board holds the files this app copied, that copy's Cut stays a move. Files another app put there are copied. With no files, Paste falls back to this app's own copy, or says "Nothing to paste. Copy files first." | `system.rs` (`board_item`, `board_files`, `paste_plan`), `ops.rs` (`copy_selected` queues the write, `paste` queues the read) |
| `search-index` (core search index, `bifrost_app::quicksearch`) | core-spotlight "Searchable items and attribute sets" (Partial) | A document opened from Files is indexed with its name as title, its folder as description and its file URL as content URL, under the domain `is.olibuijr.files.desktop`. Folders are not indexed. Search then finds the document by its words. | `system.rs` (`index_document`), `window.rs` (`open` queues it) |
| `dock` (`bifrost_app::dock_tile`) | appkit-user-interface "NSDockTile" (Partial) | A copy, move or duplicate shows its progress as a thin bar on the Files Dock icon, sent only when the fraction moves by at least 1 point (0.01). The bar clears when the job ends. Duplicate and copy jobs count; trash, compress and extract do not. | `system.rs` (`track_dock`, `end_dock`, `dock_step`), `main.rs` (`tick`) |

The app keeps its own service state in `SystemState` (`system.rs`). Handlers that see a user action only queue an
`Effect`; `SystemState::flush` sends the queue after the input (`App::event` through `flush_services`) or the tick. Calls
that expect a reply are remembered by request id and matched in `App::service_reply`. The service channel is untouched.

## How a user sees it

- Select one or more files and press Space: a card with the file's picture or text (or the name, if there is no preview)
  opens over the desktop. Left and Right change the item; the selection in Files follows the card. Space or Esc closes it.
- Select files and press Copy (or Cut), switch to another app that pastes files (or to Files in another folder), and press
  Paste. The files are copied (a Cut from Files still moves).
- Open a document (Enter or double click). It becomes findable in Search by its name and folder words.
- Start a copy of several files into a folder with a lot of data. A bar sweeps the lower part of the Files Dock icon and
  disappears when the copy ends.

## How to verify on the test PC (Midget only)

Midget is the only test machine (no VM). Build the fast shell and push it, then publish and update as usual
(`cargo build --offline --profile fast --bin shell --features desktop-wayland` in `springboard/`, then
`tools/midget-push-shell.sh --no-build shot.png`, then `bifrost pc publish` and `bifrost pc midget ota` until
`phase=accepted`). Then:

1. Preview. Open Files on a folder with a picture and a text file. Select both, press Space: the panel shows the item
   with the focus (`bifrost pc midget screen /tmp/ql.png`). Press Right: the panel shows the second item and the Files
   row selection follows it. Press Esc: the panel closes. Check Light and Dark (`bifrost pc midget appearance light|dark`).
2. Preview fallback. An older shell answers "no service" to the calls; the in-app preview then opens with Space. (Test on
   an older build only if one is at hand.)
3. Pasteboard, inside Files. Select a file, Command-C, go into another folder, Command-V: the file appears there. Cut
   (Command-X) and Paste moves it.
4. Pasteboard, across apps. Select a file in Files and Command-C. In another app that reads the board (a text field
   cannot take it; use an app with a file paste, or read the board with `pasteboard` `types` from an app), the file URL
   is listed. The reverse: copy a file in another app and Paste in Files copies it.
5. Search. Open a text document from Files. Open the desktop Search panel and type its name: the document is listed under
   Documents. Open a folder: it is not indexed.
6. Dock progress. Copy a large folder (a few hundred MB) into another folder. While it runs, the Files Dock icon shows a
   bar that moves; `bifrost pc midget screen` during the copy. After the copy the bar is gone.
7. Log check: `bifrost pc control call` on Files shows no errors from these calls.

## Host tests (run by the lead, not in this round)

`cargo test -p bifrost-files system` (from `springboard/`): file URL encoding and decoding, refusals of other schemes and
broken escapes, the board write (checks the `ServiceCall` message for `pasteboard`/`write`), the preview `show` call and
its waiting reply, the `close` call, the search index message under the app domain, the Dock `progress` call, the Dock
step threshold, the paste plan (own copy, other app's files, fallback, nothing), and `board_files` on a real temporary
file. The tests use `Ctx::testing` and `take_sent`, as `springboard/sdk/src/service.rs` does, with the font the existing
Files tests load from the packed `springboard/assets/SFUI.ttf` (Inter, SIL OFL 1.1).

## Official behaviour relied on

- The pasteboard holds several representations per item and file URLs (`public.file-url`) for files; the parity rows cite
  `ev_a56d232a88893f8c75a787655b47c8fcee754bfbfca75a880b0ce1fff09c3d10` (NSPasteboard collection page) and the
  `writeObjects(_:)` page read with `tools/parity/docc.py appkit/nspasteboard/writeobjects(_:)` (writes an array of objects).
- The Dock tile progress: `ev_9edad8ae0c9577f2271dce2edb1d93b8fcd80f734e1dce0e9ecaaadb07d0a23b` (NSDockTile) as cited in
  `docs/parity/wire-svc-dock.md`. This round did not re-read it.
- Core search indexing: `indexSearchableItems(_:completionHandler:)` read with `tools/parity/docc.py
  corespotlight/cssearchableindex/indexsearchableitems(_:completionhandler:)`: "Adds or updates items in the index".
- Preview panel: `ev_053580a3b907fd4d4e73ae530fae08c6434f6a5e8c7ceda6695abe5d67d9af7f` (see `docs/parity/wire-svc-quicklook.md`).

## Decisions and limits

- Paste reads the board on every Paste. There is no `changed` event subscription: the paste decision needs the content, not
  the change count. A Paste in a folder costs one service call.
- The board write carries at most 16 items (the shell's cap); a longer selection copies the first 16 to the board. The
  internal Copy and Cut hold the full selection, so pasting in Files is complete.
- Typed text on the board is not pasted: a folder takes only files. Paste of text falls back to the app's own copy or to
  the "Nothing to paste" notice. No text file is created.
- Quick Look stepping is by the shell's arrows. The app does not drive the panel with its own keys while it is open.
- The preview panel is drawn only on the desktop (see `docs/parity/wire-svc-quicklook.md`); a full-screen Files does not show it.
- Indexing is by opened documents only. Documents found by browsing, and folder contents, are not indexed. An indexed item
  is not removed when its file is deleted (the item expires with the shell's index rules, or the app's later delete).
- The search-index ranking service (`search_ranking`) is not used by Files; Files only feeds the index.
- Dock progress is a bar on the icon; the Dock tile's own size and owner are not used (see `wire-svc-dock.md`).
- Copy and paste do not yet use file coordination (see `docs/parity/wire-app-files.md`, "Coordination limits").
- Not built. The unit tests in `springboard/apps/files/src/system.rs` are written against the SDK's public API read from
  `springboard/sdk/src/` and the shell's replies in `springboard/src/shell/services/pasteboard_types.rs`. If the build finds
  a mismatch, it is in the glue, not the service.
