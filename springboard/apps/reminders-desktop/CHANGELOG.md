# Changelog

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
