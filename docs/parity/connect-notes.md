# Connecting Notes to the system services (connect/notes)

Notes (`springboard/apps/notes-desktop`, crate `bifrost-notes`, version 1.10.0) now uses three shell services through the
service channel. The glue is in `src/notes/system.rs` (pure functions and the service calls, with tests) and
`src/main.rs` (the app hooks). Nothing is called per frame: each call follows a user action or a change in the notes.
Not built in this round (the lead builds once).

## What is connected

| Service | Matrix slug | What Notes does | Where |
|---|---|---|---|
| `pasteboard` (typed items, `general` board) | `core-transferable` ("Copy and paste of typed items"), `application-services` ("Pasteboard") | Cmd+C with text selected in the editor writes one item with two representations: `public.rtf` (bold, italic, code, list markers) and `public.utf8-plain-text` (the markers stripped). Without a selection the chord is left alone. | `system::copy`, `copy_items`, `rtf`; `main.rs` `KEY_C` chord; `Notes::selected_markdown` in `notes/mod.rs` |
| `widgets_timeline` | `widgetkit` ("Timeline updates") | One widget timeline, kind `recent-notes`: the three newest notes (title "Recent Notes", the titles one per line), or "No notes yet". Policy `never`: the app reloads on a change and when the shell asks. | `system::recent_titles`, `widget_entries`, `reload_widget`; `main.rs` `sync_widget`, `service_event` (reload event) |
| `search-index` (search ranking) | `core-spotlight` ("Searchable items and attribute sets") | Not a new call. Notes already indexes each note through Core Search (1.8.6); the index text is now the whole note (up to 4000 chars), not three lines. The shell ranks those items for the Search panel. | `system::index_body`; `main.rs` `sync_index` |

## How a user sees it

- **Copy as rich text.** Select text in a note and press Cmd+C, then paste into a rich-text app. Bold and italic come
  through; apps that read only plain text get the plain copy with the markers removed.
- **Widget.** Open Notification Centre. The widget area shows "Recent Notes" with the three newest note titles, or "No
  notes yet" when there are none. It changes when a note is created, renamed, moved to the top by editing, or deleted.
- **Search.** Search for a word that appears in the body of a note (not only the title) and the Search panel lists the
  note under Documents. Opening the result starts Notes on that note (existing behaviour).

## How to verify on the test PC (Midget)

1. Publish and update Midget (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Host tests (the lead runs these once): `cargo test -p bifrost-notes` in `springboard/apps/notes-desktop`. The new tests
   are in `src/notes/system.rs`: `rtf_escapes_controls...`, `rtf_carries_bold...`, `a_copy_puts_rtf...`,
   `the_widget_lists...`, `the_widget_entry...`, `a_reload_goes_to...`, `the_index_text...`.
3. Copy: open Notes, create a note with `**bold** and *italic*` and a line `- item`. Select the text with the arrow
   keys and Shift, press Cmd+C. Open a text field in another app and paste: the text reads "bold and italic" and the
   list line reads "• item".
4. Widget: create two notes with different titles. Open Notification Centre and check that "Recent Notes" lists them
   newest first. Delete one: within about 3 seconds the widget drops it. With no notes it reads "No notes yet".
5. Search: type a word that occurs only in the body of a note into the Search panel. The note appears under Documents.
6. Screenshot: `bifrost pc midget screen /tmp/notes-connect.png` (window-local taps with `tap-window Notes X Y`).

## Gaps

- **Copy reaches other apps as plain text only.** The `general` board is the shell's clipboard and keeps one
  representation per item (the shell does not keep item grouping, see `wire-svc-pasteboard.md`). The rich text is in the
  service's write, but a paste elsewhere takes the plain text. Named boards keep both representations.
- **Copy is a keyboard chord only.** Edit > Copy in the shell's menu does not reach the app (the shell has no copy command
  for apps), and Cut and Paste are not connected. A drag of selected text does not carry the rich text.
- **No paste of rich text into Notes.** Pasting reads the shell's clipboard as before; Notes does not read the `pasteboard`
  service yet.
- **`search-index` expiry is not used.** Notes' trash is already removed from the index when a note is deleted, and
  Recently Deleted notes are purged after 30 days by the app (`folders::purge`). No note has an expiry date, so the
  `expire` method has no use here. `search_ranking::search` is not used either: the Search panel ranks the notes, and
  Notes' own list search stays synchronous.
- **The widget shows titles only.** The entry has no thumbnails, no per-note tap target (the shell gives the widget no
  action to open a note), and the policy is `never`: the widget does not refresh by itself while Notes is closed; it
  keeps the last list until Notes runs again.
- **Widgets and index state are in memory.** After a shell restart the widget is empty until Notes runs and changes or
  the shell asks again. The Core Search index is rebuilt from the notes on the first sync.
- **Service errors are logged only.** An older shell answers "no service ..." and the app logs it through `ctx.log`
  and carries on without the widget or the rich copy's effect on other apps.

## Files

- `springboard/apps/notes-desktop/src/notes/system.rs` (new): copy items and RTF, recent titles, widget entries, index text, tests.
- `springboard/apps/notes-desktop/src/notes/mod.rs`: `pub mod system`, `Notes::selected_markdown`.
- `springboard/apps/notes-desktop/src/main.rs`: `sync_widget`, Cmd+C chord, `service_event`, `service_reply`, index text.
- `springboard/apps/notes-desktop/CHANGELOG.md`: 1.10.0 lines (the version was already bumped this round).
