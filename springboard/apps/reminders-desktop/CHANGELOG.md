# Changelog

## 1.19.0 — 2026-10-17

- Core Search indexes every reminder with a title: the title, and the first lines of its notes. Edited reminders are indexed again, deleted or untitled ones leave the index, and a Search result opens the reminder in the list.

## 1.18.0 — 2026-10-17

- A reminder's row shows its notes with the Markdown marks taken away (bold as plain words, bullets as bullets), through `format::preview` and the shell's `md::to_plain`.
- A reminder can carry a place (a circle of latitude, longitude and radius with an arriving or leaving trigger). The place is stored as a 20th field in the store, which moves to format v4 (v1 to v3 still load; older versions refuse a v4 file). `place::Watch` reports the reminders whose place was entered or left. The form does not edit places yet and the location watch is not called, so no alert fires yet. The manifest has the location privacy purpose string.

## 1.17.0 — 2026-10-17

- Format menu (Bold, Bullet List, Numbered List) writes Markdown marks into a reminder's notes while the Notes field has the focus; a command again takes the mark away. The notes are stored as Markdown text and shown as typed, not rendered.
- Scripting: `tell app "Reminders"` can `make new reminder with properties {name, due}` and `make new list with properties {name}`, and get the name, id, notes, completed, flagged, priority, due day and list of reminders (`scripting.rs`, `[[script.*]]` in the manifest). Setting properties and deleting reminders from a script is refused.

## 1.16.0 — 2026-10-14

- Settings: the all-day alert time offers every half hour from 6:00 to 20:00 (it offered ten fixed times).
- File > Print List prints a smart list (Today, Scheduled, Flagged, All, Completed, or a custom smart list) as a flat Markdown checklist under its name. Search results are not printed yet.
- File > Save List As... writes the list's Markdown checklist to a file chosen in the save panel, beside Export List.

## 1.15.0 — 2026-10-14

- View > Sort By, View > Show Smart List and Hide All Subtasks are saved in `settings.tsv`: the sort order, the hidden smart list tiles and the folded subtasks are the same after a restart.
- A Markdown checklist (`.md`) opened or shared from Files is imported as a new list, the same as File > Import List. Reminders is an Open With choice for Markdown files.
- Removed two unused copies of the mark-due, template, grocery and time zone rules; the list uses the ones in `organize.rs` and `zones.rs`.

## 1.14.0 — 2026-10-13

- Mark Due Date As (Reminder menu): Today, Tomorrow, This Weekend and Next Week set the selected reminder's due day and keep its time. Mark All Overdue as Today moves every open overdue reminder in one undo step.
- Indent and Outdent (Cmd+] and Cmd+[) nest the selected reminder under the one above it, or lift a subtask back to the top level. View > Hide All Subtasks and Show All Subtasks fold every parent at once.
- Sections: a list can be split into named sections (List > New Section). Choose a section header to rename or delete it, use Reminder > Move to Section, or drag a reminder onto another section. Deleting a section keeps its reminders.
- List > Auto-Categorize sorts the loose reminders of a list into grocery sections (Produce, Dairy and Eggs, Bakery and so on) from the words in their titles, reusing sections you already have.
- List groups: List > New List Group makes a collapsible group in the sidebar; the list form has a Group row to put a list in one. Deleting a group keeps its lists.
- Pinned lists: List > Pin List (or the Pin in Sidebar switch in the list form) adds a tile for the list beside the smart lists. Up to eight lists can be pinned.
- View > Show Smart List hides or shows each smart list tile in the sidebar. View > Sort By offers the sort orders from the menu bar, and a new Creation Date order lists the oldest reminders first. Reminders now record when they were created.
- Image attachments: a reminder's details have Add Image, which opens a file panel for a PNG or JPEG (up to 12 MB and 12 million pixels, four to a reminder). The pictures show as thumbnails in the details, the list row says how many a reminder has, and Remove Last Image takes one off. They are copied into an `attachments` folder beside the reminders file; Cancel deletes the ones added in that visit, and removing one and pressing Done deletes its file unless a duplicate still uses it.
- Urgent alarm: Reminder > Mark as Urgent (or the Urgent Alarm switch in the reminder's details, which needs a date). Once an urgent reminder is due, its alert repeats every minute until you complete it, and stops on its own after a day.
- Time Zone in a reminder's details: choose from a short list of zones, and the due time is read on that zone's clock, with its daylight saving, instead of the device's. The list shows the place next to the time. Same as Device is the default.
- Smart lists you define: File > New Smart List opens a form with a name, a colour and rules for the date (overdue, today, next 7 days, later, no date), time of day, priority, flag, one tag and one list. A smart list collects the open reminders that satisfy every rule you set, shows its count in the sidebar, and is edited or deleted from Show List Info.
- Settings (Reminder > Settings, Cmd+,): the list new reminders go to, the time an all-day reminder alerts (6:00 to 20:00; it was fixed at 9:00) and a switch for the Dock badge. They apply at once and are kept in `settings.tsv`; Undo does not touch them. The Today widget and the scheduled alerts follow the all-day time.
- Templates: List > Save List as Template keeps a list's reminders without dates or completion; New List from Template makes an open list from one, and Delete Template removes it.
- File > Import List reads a Markdown checklist (the Export List format, with subtasks, due days and `## section` headings) into a new list. File > Print List prints the open list. Export List now writes each section as a `## heading`.
- Quick Reminder: a menu bar extra with New Quick Reminder (opens the new reminder form) and Add Reminder from Clipboard, which reads the copied text as a quick-add line such as "pay rent every month !!" and adds it to the default list.
- Shortcuts: new Add Quick Reminder, Create List, Open List, Open Smart List and Search Reminders actions next to Add Reminder.
- The store keeps list groups, sections, templates, smart lists, pinned lists, creation dates, time zones, urgent marks and picture names in the same file (format v3). Files from older versions still open and are rewritten in the new format on the next change. An older version of Reminders does not read a v3 file: it reports that it cannot read it and leaves it untouched.

## 1.13.3 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.13.2 — 2026-10-12

- Removed `model::outdent`, which nothing called. Make Regular Reminder on the Indent row already clears the draft's
  parent, so that is the one outdent path.

## 1.13.1 — 2026-10-12

- Removed the unused procedural icon and the unused badge drawing, and the state dump for tests that are not built; the app uses its icon.png.

## 1.13.0 — 2026-10-12

- Tags: a word written as `#tag` in a reminder's title or notes is a tag, such as `#home` or `#Work`. Tags match without case, so `#Home` and `#home` are one tag.
- The sidebar has a Tags section under My Lists, listing each tag with the number of open reminders that carry it. Choose a tag to see those reminders, with the same show and hide rules for completed ones as a list.
- The Tags section appears only when a reminder has a tag, so the sidebar is unchanged until you use one.

## 1.12.0 — 2026-10-12

- Edit > Undo and Edit > Redo (Cmd+Z and Cmd+Shift+Z) now work on every change to your reminders: completing or reopening one, saving a new or changed reminder, duplicating, moving by drag, and creating or renaming a list. Undo and Redo show the change's name in the Edit menu, such as "Undo Mark as Completed".
- Undo steps back through the changes in order, and Redo steps forward again. Deleting, clearing completed and flagging still work as before, and they are part of the same history.
- A new change after an undo drops the steps you undid. Edit > Undo is available on the list, not while a reminder's details are open.
- The smart lists (Today, Scheduled, Flagged, All and Completed) are now defined as queries over the reminders, with the same results as before. Their counts and lists update as soon as a reminder changes.

## 1.11.0 — 2026-10-12

- A new reminder's details now show a short tip under Due: set a due date and a time, and the alert still shows when Reminders is closed. Setting a date, or tapping the tip, dismisses it for good.
- The window background is the content grey in Dark (white in Light) instead of black, so the list and the details sit on the same surface.

## 1.10.0 — 2026-10-12

- Due-date alerts are now scheduled with the system. Each upcoming alert (and each early alert) is handed to the shell as a notification with a time trigger, so it shows on time even when Reminders is closed, and it is replaced or removed when the reminder changes. The app keeps up to 32 alerts pending and re-plans when the list changes, when the time zone changes, and at least once an hour.
- Repeating reminders schedule their next alert as soon as they are completed, through the same path.
- Due-date alerts go through the shell's `notification-schedule` service. A shell without it falls back to the older notification path, so alerts keep working.
- The Dock badge (the count of open reminders due today or earlier) is set through the `dock` service, with the older badge message as the fallback.
- Today widget: a Notification Centre timeline lists today's open reminders and overdue ones, with a summary row. It is updated when reminders change, at each shell reload request, and at the next midnight.

## 1.9.4 — 2026-10-11

- Undo records every delete, clear and flag change on the shared `UndoManager` (`bifrost_app::controls::undo`), named after the change. The Undo bar still offers the latest one for its usual time; its behaviour is unchanged.

## 1.9.3 — 2026-10-11

- The toolbar search is a visible search field on the first screen (as the stock toolbar search), not a collapsed button, so the field is published as an accessibility text field and is one click away.
- Compliance gate: layout values use the SPACE_* tokens and named layout constants, the alert clock conversion no longer reads as a screen resolution, the icon uses named colours, and the switch knob and the tinted list title carry justified compliance-allow notes.

## 1.9.2 — 2026-10-11

- One app menu, Reminder (New Reminder, New List, Delete Reminder, Mark as Completed, Find). The shell's File, Edit, View, Window and Help menus are no longer repeated.
- The sidebar's smart lists, My Lists rows and Add List are published as accessibility controls with their labels and counts.
- Delete actions use the destructive red; links (calendar arrows, Undo, Duplicate, Make Subtask, stepper glyphs) and the Switch's on state use the one tint.
- Corner radii come from the prefs layout tokens (cards 13 pt, swatches 5 pt) and scale with the window.
- Sudden termination: the app may be ended at once on quit, logout or shutdown, except while an editor holds an unsaved draft.
- Needs system 0.5.7 (`min_shell`), the first with sudden termination.

## 1.9.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.9.0 — 2026-10-10

- Main menu in the menu bar: New Reminder (⌘N), New List (⇧⌘N), Delete Reminder, Find, Mark as Completed.
- App Intent: Add Reminder (title, optional due date).

## 1.8.0 — 2026-10-10

- Looks like Reminders on the Mac (Golden Gate): a full-size window with the traffic lights over a sidebar holding the smart-list tiles two to a row (Today, Scheduled, Flagged, All, Completed, in their colours with counts), My Lists as a source list with colour dots and counts, and Add List at the bottom. The open list sits beside it with its large coloured title and open count, in place of the iPad screens.
- Unified toolbar: Export List, View Options, Show List Info, New Reminder and a search field; a reminder's details and list info open beside the sidebar with Cancel and Done in the toolbar. Every button is published as a control.
- Follows Light and Dark: every colour is a system `Pair` token and the type uses the macOS text styles.
- Needs system 0.4.84.

## 1.7.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.7.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 41 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.7.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.7.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.6.0 — 2026-10-09

- The Dock icon shows a red badge with the number of open reminders due today or earlier (99+ above 99), as in Apple Reminders. Needs system 0.4.60.

## 1.5.0 — 2026-10-08

- Options > New Reminders Go To picks the list for reminders made from a smart list; Automatic keeps the first list.
- Options > Export List writes the open list as a Markdown checklist to Documents/Reminders Export, with subtasks and due dates.

## 1.4.0 — 2026-10-08

- The undo bar names the change: Flagged or Unflagged, with the reminder's title.

## 1.3.0 — 2026-10-08

- Cmd or Ctrl+Backspace deletes the reminder selected in the list.
- Escape leaves a focused field, then cancels the open editor.

## 1.2.0 — 2026-10-08

- Cmd or Ctrl+N makes a new reminder; with Shift it makes a new list. Cmd or Ctrl+F searches.
- Cmd or Ctrl+Return saves the open reminder. Cmd or Ctrl+Z undoes the last delete.

## 1.1.0 — 2026-10-08

- Up and Down select a reminder; Enter opens it; Space completes it or marks it open again. Left and Right move the caret in text fields.
- Duplicate in the editor copies the reminder directly after the original, as an open item.
- Options > Clear Completed removes the completed reminders in the open list. Undo restores them.
- Swipe a reminder to the right to complete it.

## 1.0.0 — 2026-10-08

- Reminders for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Deliver permitted notifications through the shell while the app is running.
