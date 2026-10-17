# Changelog

## 1.21.0 — 2026-10-17

- Math: `$x^2$` in a note is typeset inline, in place of its source (fractions, roots, scripts, sums and integrals with limits, Greek letters, matrices). A `$$...$$` paragraph is display math, centred on its own line. The source stays in the note file; a formula that does not parse shows as text. Typesetting is in `bifrost-render` `math` (textarea.rs `formula`, md.rs `Style::Math`).

## 1.20.0 — 2026-10-17

- Collapsible sections: a Title, Heading or Subheading line shows a disclosure chevron. Tapping it folds or unfolds the lines up to the next heading of the same or a higher level; the editor takes no height for hidden lines (textarea.rs `set_hidden`). Format > Collapse Section, Expand Section, Collapse All Sections and Expand All Sections do the same from the keyboard. Folds are saved per note in `.springboard-notes-folds`, and entries of deleted notes are dropped when it is saved. The note text never changes.
- Gallery: View > as List / as Gallery and the list toolbar toggle draw the note list as a grid of cards (first picture or a text preview, title and date). Columns follow the list width (`notes/gallery.rs`). Pictures decode off the UI thread. Tap a card to open it.

## 1.19.0 — 2026-10-17

- Edit > Speech: Start Speaking speaks the selection of the open note (else the whole note) through the shell speech service; Stop Speaking ends it.

## 1.18.2 — 2026-10-14

- Use named system black and white colors for sketch ink and paper pixels.

## 1.18.1 — 2026-10-14

- Send print jobs through the shared Utility dispatch queue so the app UI never creates a raw worker thread.

## 1.18.0 — 2026-10-14

- Note links open their note: the manifest registers the notes scheme, and an Open event with a notes://note/<id> link shows that note when it exists (src/main.rs, Input::Open). The lab machine has not yet verified the routing.

## 1.17.0 — 2026-10-14

- Attachment browser model (notes/attachments.rs): every attachment of every note, newest note first, with its folder and caption. The browser grid is not drawn yet.
- Rename attachment: a caption saved per file in a sidecar file (notes/attachments.rs, set_caption), so file names on disk never change. The caption is not shown yet.
- Settings model (notes/settings.rs): default text size, dark note background, new note start and attachment tile size, saved beside the notes. There is no Settings pane yet and the values do not change the editor.
- Note links (notes/note_url.rs): notes://note/<id> links are built and read back with the id validated. The scheme is not registered and links do not open notes yet.
- Not in this version: the attachment grid, caption display, Small and Large tiles, the Settings pane, the note-link handler, speech, gallery view, separate note windows and locked-note changes are still missing.

## 1.16.0 — 2026-10-13

- Import to Notes also reads HTML and RTF files, converting headings, emphasis, lists and paragraphs to Markdown (notes/import.rs); a file with no text is skipped and logged.
- Format menu: Increase/Decrease Indent (Cmd+] / Cmd+[), Move Line Up/Down (Ctrl+Cmd+Up/Down), Mark All Checked and Unmark All; the same commands are in the editor More menu.
- Pipe tables get Add Row Above/Below, Add Column Left/Right, Delete Row/Column and Convert Table to Text (notes/table.rs), shown in the editor More menu while the caret is in a table.
- The list menu can sort Newest First / Oldest First (A to Z / Z to A for titles); the choice is saved with the other list settings.
- Note > Recent Notes lists the ten newest notes and opens the one chosen.

## 1.16.0 — 2026-10-13

- Format menu (the Format entry in the More menu of an open note, and a Format menu in the menu bar): paragraph styles Title, Heading, Subheading, Body, Monostyled (four leading spaces), Block Quote (`> `); lists Bulleted, Dashed (an en dash), Numbered and Checklist; character styles Bold, Italic, Underline (`++text++`), Strikethrough (`~~text~~`), Highlight (`==text==`) and Inline Code. The editor draws strike-through, underline, highlight and the quote bar. Cmd+U, Cmd+Shift+X and Cmd+Shift+H apply the new character styles.
- Indentation: Tab and Shift+Tab (also Increase and Decrease Indent in the Format menu) move list lines one level in or out. Ctrl+Cmd+Up and Ctrl+Cmd+Down (or Move Item Up and Down) move a list item, with the items nested under it, past its neighbour.
- Checklists: Mark All and Unmark All in the Format menu tick or untick every item in one undo step.
- Tables: with the caret in a table, the Format menu adds a row above or below, a column left or right, deletes the row or column, or converts the table to text with tab-separated cells.
- Tags: a `#word` that starts a word is a tag. The sidebar has a Tags section with a count per tag, the list menu lists them too, and choosing one shows the notes that carry it. Notes in Recently Deleted are not counted.
- Group by Date (list menu): the list is cut into Today, Previous 7 Days, Previous 30 Days and month sections when sorted by a date. Pinned notes keep their own section above. Search and the title sort show the list without date sections.
- Newest First and Oldest First (list menu) set the direction of the date sorts; the choice is remembered.
- Math Results: turn it on in the Note menu or the Format menu, and typing `=` after a calculation (`12 * 3 =`) writes the answer. Calculate Math Results answers every such line in the note at once. It reads `+ - * / % ^` and brackets.
- Add Link (Cmd+K, Format menu): asks for a web address and turns the selection into a `[text](address)` link.
- Note menu: Copy as Markdown, Paste as Markdown, Paste and Match Style (removes the Markdown marks of the pasted text), Math Results and a Recent Notes submenu with the five newest notes.
- Import to Notes accepts HTML and RTF files and converts them to Markdown: headings, bold, italic, links, lists, quotes and code stay; images, colours and fonts are dropped.
- Menu bar extra: Resume Last Quick Note reopens the note New Quick Note made last. The list menu has a setting to make New Quick Note resume it too.
- Shortcuts: new intents Open Note (by title or id) and Open Folder.
- Not in this version: tags are not completed while typing and cannot be combined in a filter; tables are still not drawn as a grid; Tab and Shift+Tab, the new chords and the menu-bar entries have not been tried on Midget; locked notes, the attachment browser, the gallery view, collapsible sections, separate note windows, Speak, a settings window, user-made smart folders and note links by URL are still missing.

## 1.15.0 — 2026-10-13

- Note > Print... (Cmd+P) prints the open note as plain text in Helvetica, wrapped to the page, through the shared print
  operation. The job is submitted on its own thread.

## 1.15.0 — 2026-10-13

- Share as PDF (the More menu in the editor) writes the note as a PDF: the title in bold, the body in Helvetica, wrapped and paginated. Share sends the file through the save panel, as Markdown and text do.
- Insert Table (the More menu in the editor) puts a header row, its rule and an empty row at the caret as a Markdown table, typed in place.
- New Quick Note in a menu bar extra starts a note from the menu bar, the same as Cmd+N.
- Not in this version: a note cannot be locked with a password; tags are not listed or filtered as a group (a `#tag` is found by search as text); tables are not drawn as a grid in the editor, and the PDF shows each row on one line; the menu bar extra's symbol has not been checked on Midget.

## 1.14.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.14.0 — 2026-10-12

- The remote state dump is now served as the notes.state control method, so an agent can read the open screen and state without the screen.

## 1.13.0 — 2026-10-12

- Replace (Replace in Note in the More menu of an open note) changes the text you searched for. Type the replacement in the card at the top of the note: Replace changes the selected match and moves on to the next one, Replace All changes every match in one step (one Undo takes it back). Enter does Replace All, Escape closes the card. The card needs find text first: type it in the search field, then choose Replace.
- Replace matches the find options: Match Whole Word in Find applies to Replace too.
- Smart Folders in the sidebar: Pinned, Edited This Week (changed in the last seven days), Checklists (notes with a checkbox line) and Photos (notes with an attached picture). Each shows its count. The list menu lists them too. A smart folder only shows notes; nothing is moved or written, and notes in Recently Deleted never appear.
- Not in this version: the find bar has no replace field of its own (Replace opens the card instead), replacement is not offered while a spelling check is shown, and smart folders cannot be made or edited by the user.

## 1.12.0 — 2026-10-12

- Insert Sketch (the More menu in the editor) opens a sketch pad over the window. Drag with the mouse or a pen to draw black lines. The header has Cancel (closes without saving), Undo (removes the newest line) and Done (saves the sketch and puts its picture at the caret, where it shows inline like a photo).
- Each sketch is kept beside the note twice: as a picture (a PNG) and as its drawing (a `.sketch` file with the line data). Moving, duplicating, recovering or deleting a note carries both files along.
- Only pen lines are offered for now: no colours, marker or pencil, no erasing a single line, and no reopening a sketch to edit its lines. Pressure from a stylus is not recorded yet.
- The note's background is now the content colour (white in Light, dark grey in Dark), the same as the rest of the content area, instead of the pure black used by the detail column.
- Not in this version: checklist boxes keep their current drawing; symbol rendering modes are not applied to them yet.

## 1.11.0 — 2026-10-12

- Check Spelling and Links (the More menu in the editor) now also flags street addresses and flight numbers, such as "FI 552" or "BA117", as one item each, next to the links, dates and phone numbers. Use the find arrows to step through them.
- Not in this version: flight numbers and addresses are not yet underlined in the note body (links, dates and phone numbers are), and tapping them does nothing. The checklist boxes keep their current drawing; symbol rendering modes are not applied to them yet.

## 1.10.0 — 2026-10-12

- Find: Match Whole Word in Find (the More menu in the editor) makes a search skip matches inside a longer word, so "rust" does not count "rusty". The matches come from the shared find model (`bifrost_app::controls::textfind`).
- Check Spelling and Links (the More menu in the editor) flags the misspelled words of the note, merged with its links, dates and phone numbers. Use the find arrows to step through them. Spelling uses the English or Icelandic word list, chosen from the note's language; other languages get only the links, dates and phone numbers.
- The date line under the title shows the note's language, detected from its text (for example "Icelandic").
- Replace and the list styles beyond bullets and numbers are not in this version yet.
- Copy (Cmd+C) with text selected in a note puts the selection on the pasteboard as rich text (bold, italic, code and the list markers), with the plain text as the fallback for apps that do not read rich text.
- A widget lists the three newest notes (Recent Notes, or "No notes yet"). It updates when the newest notes change, and when the desktop asks for it.
- Search finds more of each note: the text of the whole note is indexed (up to 4000 characters), not only its first three lines.

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
