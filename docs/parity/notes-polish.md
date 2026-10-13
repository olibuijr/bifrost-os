# Notes polish (slug notes-polish)

App: `springboard/apps/notes-desktop` (bifrost-notes 1.15.0). Scope from the round-75 slice: folders and tags,
checklists, tables, inline attachments, pin, lock with password, search, share or export as PDF and Markdown,
quick note from the menu bar.

## Reference evidence

- Notes app bundle (shipped macOS 27 files): `~/Projects/macos-ref/extract/ref27/System/Applications/Notes.app/Contents/`.
  The main window nibs are under `Resources/Base.lproj/` (for example `ICMMainWindowController.nib`,
  `ICMFolderListController.nib`, `ICMAttachmentBrowserController.nib`, `ICMInlinePopupButton.nib`).
- REA `inspect_plist` and `decode_interface_builder` were not run against these nibs: REA reported no open
  binary, so the menu contents were not decoded. Behaviour was taken from the existing app and the feature
  list, not from decoded nib connections. This is an open gap for the lead to re-check.

## Per feature

| Feature | API / BifrOSt path | State | Midget check |
|---|---|---|---|
| Folders | `notes/folders.rs`, sidebar in `notes/mod.rs` | Existed before this round, unchanged | Yes, open a folder |
| Tags | text `#tag` in `notes/store.rs` (`find_all`) | Not changed. A `#tag` is found by search as literal text. No tag list or tag filter. | Decide if a tag list is needed |
| Checklists | `springboard/shell/md` checkbox lines, `notes/mod.rs` | Existed, unchanged | Tap a box |
| Tables | Insert Table, `notes/mod.rs` `insert_table` | New: inserts a Markdown pipe table at the caret. The editor shows it as text, not a grid. | Insert, type cells |
| Inline attachments | `notes/attach.rs`, `notes/picker.rs`, `notes/sketch.rs` | Existed (photos, sketches), unchanged | Attach a photo |
| Pin | `notes/store.rs` pins, Pin Note in the editor menu | Existed, unchanged | Pin and reopen |
| Lock with password | none | Not done. No password store and no unlock UI. | Not applicable |
| Search | `notes/store.rs` `search_ranked`, find bar | Existed, unchanged | Search |
| Share as Markdown and text | `Act::ShareMd`, `Act::ShareText` | Existed, unchanged | Share |
| Share as PDF | `notes/pdf.rs` (`render`), `store::export_file`, `Act::SharePdf` | New: pure PDF writer (standard Helvetica, WinAnsi, bold title, wrapped, paginated). Shared through the save panel like Markdown. | Open the .pdf in a viewer |
| Quick note from the menu bar | `apps/notes-desktop/src/main.rs` `quick_note_item`, `status_item_action`; SDK `sdk/src/status_item.rs` | New: a menu bar extra with New Quick Note, which runs the New Note action. The extra's symbol `square.and.pencil` is not checked on Midget. | Check the extra appears and creates a note |

## Migrations and deletions

- `store::export_md` now calls the new `store::export_file`. Behaviour and file name are unchanged.
- No old code was removed in this round.

## Tests

Unit tests added in `notes/pdf.rs` (xref offsets, escaping, wrapping, tables, pagination, empty note),
`notes/mod.rs` (`notes_export_as_markdown_or_plain_text` extended with the PDF share,
`insert_table_puts_a_pipe_table_on_its_own_paragraph`). The menu bar action and the status item are not unit
tested: `main.rs` has no test harness and the SDK test context is not public. Nothing was compiled or run in this
worktree (round rule: the lead builds).
