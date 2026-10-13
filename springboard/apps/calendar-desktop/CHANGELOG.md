# Changelog

## 1.14.0 — 2026-10-13

- File > Settings... (Command-comma): week start, day start hour, default event length and default alerts for timed and
  all-day events. The settings are kept beside the events; the month grid, the week view, the scroll position of Day and
  Week and new events follow them. The Day and Week zoom level is saved too.
- Go > Search Events... (Command-F) and a toolbar button: a search screen with a query field and a scrolling result
  list. Return or a click opens the event.
- Day and Week: dragging empty hour-grid space sizes a new event and opens its editor; dragging a timed event moves it
  to another time or day.
- Edit > Cut, Copy and Paste work on events (Event > Cut Event, Shift-Command-X, deletes after copying; Undo restores it).
- File > Export Calendar Archive and Import Calendar Archive...: the events and their rich data in one backup file; a
  restore keeps the previous events as events.tsv.bak.
- Calendar files (.ics) opened from Files are imported.
- Intents create-event and show-date, and the control methods calendar.state, calendar.show_date and calendar.create_event.

## 1.13.0 — 2026-10-13

- Go > Go to Date... (Shift-Command-T): a sheet with year, month and day steppers; Return shows the date, Escape cancels.
- Go > Zoom In / Zoom Out (Command-= / Command--): changes the hour height of the Day and Week views, between 24 and 96 points.
- Event > Copy Event / Paste Event (Shift-Command-C / V): the saved event being edited goes to the pasteboard as a calendar
  file with its location and notes; Paste adds the pasteboard's event to the selected day.
- File > Import...: reads a calendar file (.ics, .vcs) chosen in the open panel and says how many events were added.

## 1.12.0 — 2026-10-13

- Internet Accounts: the CalDAV accounts switched on for Calendars are read in the background (discovery, then the events
  of the last 31 days to the next year) and imported into the calendar. The password comes from the passwords group; the
  sync runs at start and every 15 minutes. Reminders over CalDAV are not read yet.

## 1.11.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.11.1

- Removed the unused import and gated the layout test helper to tests.

## 1.11.0

- Invitees: an event that came from an imported calendar file now lists who was invited, under the notes in its editor, as "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)". Each person's answer is shown: Accepted, Declined, Maybe, Awaiting reply, Delegated, Completed or In progress. Three names are listed, then "and n more". The row cannot be edited: attendance is what the file recorded.
- Invitees are kept when the calendar file is saved again, so an import does not lose them.
- Still open, listed in `docs/parity/wire6-calendar.md`: the calendar list does not group calendars by account (it shows one local calendar), an event has one alert (not several) and the repeat choices are fixed (no every N days or weeks), because the event store holds one alert and one repeat value.

## 1.10.0

- File > New Event from Text: copy a sentence and choose the command. The editor opens on a new event read from it: "Lunch with Ana next Tuesday at 12:30 at Laugavegur 5" gives the title "Lunch with Ana", the next Tuesday, 12:30 and the location "Laugavegur 5". Also understood: today, tonight (19:00 if no time), tomorrow, weekdays, "on" and "next" before a weekday, dates as 2026-10-20 or 20.10.2026, and times as 9:00, 3pm or noon. Nothing is saved until Save. With nothing copied, it opens a plain New Event.
- Event notes: a phone number in the notes (for example "+354 555 1234") is offered as "Copy +354 555 1234" under Notes. Tapping it copies the number to the pasteboard and confirms it.
- Still open, listed in `docs/parity/wire4-calendar-detect.md`: notes do not offer a new event from a date or time they mention, and the sentence parser is English only.

## 1.9.0

- Event notes: when the notes contain a street address (for example "Laugavegur 12"), the editor offers "Use ... as Location" under the notes. Tapping it fills the Location field. The address is found with the same detector the Notes app uses.
- The calendar's main content area uses the window content colour (white in Light, dark grey in Dark) instead of the pure black of the detail colour, in the month, week, day and year views.
- Still open, listed in `docs/parity/wire3-calendar.md`: Up Next does not rank by time clues (the shell timeline has no relevance field), and events have no availability (busy, free or tentative) setting.

## 1.8.4

- Alerts go to the desktop's notification schedule: each alarm of an event is one request with a calendar trigger on its weekday and time. Editing an event replaces its request; deleting it removes the request. Without the schedule service (an older shell), the app posts its alerts itself, as before.
- Up Next widget: the Notification Centre widget lists today's timed events that have not started, and moves to the next one as each starts. It refreshes when the store changes and at midnight.
- Dock: the app's Dock tile shows today's day of the month as its badge, through the dock service.
- Calendar's change counter is now the shared store-change model (the EventKit change notification): every save or import posts one change, and the alert state rescans once per change, however many saves came in between.
- Alerts, repeats and the calendar store were already wired in the app; this round adds the store-change model and its tests (`tests/store_changes.rs`). Calendar notification triggers and time-zone database pickers are still gaps and are listed in `docs/parity/wire-app-calendar.md`.

## 1.8.3 — 2026-10-11

- Compliance: the calendar list is shown on the first screen (macOS Calendar shows it by default; it still folds away below 640 pt and with the toolbar toggle), so the source list is published as Row controls in the compliance test.
- Compliance: the app icon uses the shared WHITE and BLACK tokens, not literal colours.
- Compliance: week and year view positions and the event colour bar's capsule radius are named layout tokens, not literal sizes.
- The calendar header's two-run title keeps its own draw, with a justified `compliance-allow: LAY-07`.

## 1.8.2 — 2026-10-11

- Compliance with the SDK and Golden Gate rules: the calendar list uses the calendar glyph (source list row), not a colour dot.
- Menus: an Event menu (New, Save, Duplicate, Export, Delete, Export Calendar) and a Go menu (Today, Previous, Next, Day, Week, Month, Year, Week Numbers, Calendar List). File, Edit, View and Window stay the shell's, so none is repeated. Key equivalents: Cmd+N, Cmd+S, Cmd+T, Cmd+1 to Cmd+4.
- Colours follow Light and Dark everywhere: the accent, red and destructive colours are the system `NSColor` pairs (`colors::LINK`, `colors::SYSTEM_RED`); the error line is primary text, so red is only the today marker and Delete.
- Cards (the event form's groups and the Undo bar) use the 13 pt card radius from `prefs_layout`.
- Accessibility: the month grid's day buttons and their events, the event form's fields, toggles and buttons, and the Undo bar are published with labels.
- Sudden termination: the app keeps no unsaved state beyond an open event draft, so the shell may end it at once.
- Headless compliance test (`tests/compliance.rs`): the root at 1200 x 780 points and scale 1.5625 in Light and Dark, the toolbar and the editor's controls labelled and inside the window, and the menu commands.
- The calendar is a library (`src/lib.rs`), so the test can drive it; the app binary only hosts it.

## 1.8.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.8.0 — 2026-10-10

- Looks like Calendar on the Mac (Golden Gate): a full-size window with the traffic lights in the toolbar band, no title row and no iPad navigation bar.
- Unified toolbar: New Event, the Day / Week / Month / Year segmented group, Week Numbers and Export Calendar; the header under it has the large month title with back, Today and forward capsules. Every button is published as a control.
- Month view is the Mac grid: hairline cells edge to edge, the day number at each cell's top right (today in the red circle), days of the neighbouring months greyed, and each day's events listed in its cell with "n more" when they do not fit; a click on an event opens it. The agenda under the grid is gone.
- An optional calendar list (the sidebar button) shows the local calendar with its colour.
- Follows Light and Dark: every colour is a system `Pair` token, the type uses the macOS text styles, and only today and Delete are red.
- Needs system 0.4.84.

## 1.7.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.7.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 23 padding and gap values moved to the nearest standard step (at most 2 pt).
- 4 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.7.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.7.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.
- The previous/next chevrons match the size of the + button.

## 1.6.0 — 2026-10-08

- Navigation links (Today, +, Export, Week No., the month chevrons) use the system blue instead of red. Red stays for today's date.

## 1.5.0 — 2026-10-08

- Deleting an event shows an Event deleted bar with Undo for five seconds.
- Week No. in the Month view shows ISO week numbers in a leading column; the setting is kept between runs.

## 1.4.0 — 2026-10-08

- Escape closes the event editor without saving.

## 1.3.0 — 2026-10-08

- Cmd or Ctrl+N starts a new event; Cmd or Ctrl+T goes to today.
- Cmd or Ctrl+1, 2, 3, 4 switch to Day, Week, Month and Year.

## 1.2.0 — 2026-10-08

- Enter opens a new event on the selected day.
- Export in the month view saves the whole calendar as Calendar.ics in ~/Documents/Calendar Export.

## 1.1.0 — 2026-10-08

- Left and Right move the selected day by one day; Up and Down move it by one week, when no event is open.
- Duplicate in the event editor opens an unsaved copy; Save adds it.
- Export Event in the event editor saves the event as an .ics file in ~/Documents/Calendar Export.

## 1.0.0 — 2026-10-08

- Calendar for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Route physical text, Backspace and Return keys to existing field editors.
- Deliver permitted notifications through the shell while the app is running.
