# Changelog

## 1.9.0 — 2026-10-11

- Resume: the open note and the folder view (All, a folder, Recently Deleted) come back after a restart, a reboot or an update (system 0.5.33 state restoration).
- Undo: the Edit menu says "Undo Typing" while a typing run is open, and Undo and Redo are disabled when there is nothing to undo or redo.

## 1.8.6 — 2026-10-11

- Notes are indexed with Core Spotlight (`CSSearchableIndex`): the Spotlight panel lists them under Documents, and opening one starts Notes on that note (`App::continue_activity`). New and edited notes are indexed within a few seconds; deleted notes and those in Recently Deleted are removed from the index.

## 1.8.5 — 2026-10-11

- Editor undo and redo run on the shared `UndoManager` (`bifrost_ui::undo`) instead of the text area's own edit stack: a run of typing is one named step ("Undo Typing"), the history is bounded at 50 steps and a new edit clears redo. Behaviour is unchanged.

## 1.8.4

- Main menu is one app menu, Note (New Note, Delete Note, Import to Notes, Find); the shell supplies File, Edit, View, Window and Help, so no app menu repeats them.
- Edit > Undo (⌘Z) and Redo (⌘⇧Z) come from the shell's Edit commands (`commands` and `Input::Command`), enabled while a note is open; no hand-built Undo or Redo items.
- Layout tokens for the tag and wrap-column widths; note ids use `Duration` for the millisecond conversion.
- App compliance gate: zero findings (`python3 tools/check-app-compliance.py --app springboard/apps/notes-desktop`).

## 1.8.3

- Background work uses the SDK dispatch queues (`DispatchQueue::global(Qos::Utility)`, results delivered with `DispatchQueue::main().async_`) instead of raw threads; the image and photo-picker decodes no longer ask the shell for a 30 ms wake-up.
- Edit menu: Undo (⌘Z) and Redo (⌘⇧Z) in the menu bar, the editor's own undo and redo.
- Sidebar folders and Recently Deleted are published as accessible rows.
- Radii from the Golden Gate settings tokens (`m::CARD_RADIUS`, `m::SELECTION_RADIUS`); Undo toast and bar buttons use `LINK`; swipe Delete uses `DESTRUCTIVE`.

## 1.8.2

- Logs through Unified Logging (`is.olibuijr.notes`, categories `save` and `sync`); save failures are `error` entries.

## 1.8.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.8.0 — 2026-10-10

- Main menu in the menu bar: New Note (⌘N), Delete Note (⌘⌫), Import to Notes (⌘O), Find (⌘F).
- App Intents: New Note (Spotlight shortcut), Create Note, Search Notes.

## 1.7.0 — 2026-10-10

- Looks like Notes on the Mac (Golden Gate): a full-size window with the traffic lights over a source-list sidebar of folders (All Notes, your folders, Recently Deleted, with counts), the note list beside it and the open note on the right, in place of the iPad screens.
- One unified toolbar: New Folder and Hide Sidebar over the sidebar; the list's title, note count and View Options (folders, sorting); New Note, paragraph style, checklist, bulleted list, Add Photo, Share, More and a search field over the note. The buttons are published as controls.
- Follows Light and Dark: every colour is a system `Pair` token and the type uses the macOS text styles.
- Pin Note moved into the More menu; swipe a row to pin or delete as before.
- Needs system 0.4.84.

## 1.6.1 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.6.0 — 2026-10-09

- Export: Share saves the note as a Markdown file through the system save panel (`NSSavePanel`); you choose the folder and the name, and Notes never sees the rest of your files.
- Import to Notes (Cmd+O): a text or Markdown file chosen in the system open panel becomes a new note.
- Permission `documents` (the panels); requires system 0.4.81.

## 1.5.3 — 2026-10-09

- Links in a note are underlined in the accent colour; tapping one opens it in the browser (the caret stays put).
- Requires system 0.4.71 (`detect` kit).

## 1.5.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 27 padding and gap values moved to the nearest standard step (at most 2 pt).
- 2 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.5.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.5.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.4.1 — 2026-10-08

- The note count footer is 13 pt secondary grey, as the other footers.

## 1.4.0 — 2026-10-08

- Duplicate Note in the editor menu copies the text and attached images into the same folder and opens the copy.
- Clear Completed in the editor menu removes ticked checklist items as one undo step.
- Recently Deleted rows say when the note was deleted and how many days are left before it is purged.

## 1.3.0 — 2026-10-08

- Cmd or Ctrl+B and Cmd or Ctrl+I make bold and italic in the editor.
- Cmd or Ctrl+Backspace deletes the note selected in the list.
- Escape closes the open note, or leaves the search field.

## 1.2.0 — 2026-10-08

- Cmd or Ctrl+N makes a new note and Cmd or Ctrl+F searches, on the list.
- Cmd or Ctrl+Z undoes and Cmd or Ctrl+Shift+Z redoes, in the editor.

## 1.1.0 — 2026-10-08

- Up and Down move a row selection in the note list; Enter opens the selected note. In the editor, Up and Down move the caret by line.
- After a delete, a bar reads "Note deleted  Undo" for five seconds. Undo puts the note back in the folder it came from.
- Swipe a row to the right: a half swipe reveals Pin, a full swipe pins or unpins the note.

## 1.0.3 — 2026-10-08

- Serve the app control socket (JSON-RPC): `notes.list` and `notes.get`, read-only, for agents and `bifrost pc control`.

## 1.0.2 — 2026-10-08

- Share the Notes library between tablet and desktop hosts and use the extracted UI foundations.

## 1.0.1 — 2026-10-08

- No on-screen keyboard on the desktop: typing uses the physical keyboard, which now reaches the note editor and folder names.

## 1.0.0 — 2026-10-08

- Notes for the BifrOSt desktop: the same folders, Markdown, checklists, attachments and search as the tablet app, in a window with the desktop keyboard and pointer.
