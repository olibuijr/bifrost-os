# Changelog

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
