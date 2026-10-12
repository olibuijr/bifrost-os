# Changelog
## 1.4.0 — 2026-10-12

- Stopwatch: a Copy Laps button in the toolbar puts the lap table on the clipboard as plain text (a header line, then
  each lap's number, split and total, oldest lap first). It is greyed out until the stopwatch has started.
- The window background in Dark is the content grey, not the pure black of the detail background.

## 1.3.0 — 2026-10-12

- Notification Centre shows a World Clock widget: the chosen cities and their local times, one minute at a time for the
  next hour. It updates when you add, move or remove a city, and the app refreshes it when the hour runs out.
- The Dock icon shows a progress bar while a timer runs, and bounces once when a timer finishes.
- Alarms and timers still ring through the shell; this round does not add a second notification for them.

## 1.2.3 — 2026-10-11

- Compliance: the fixed-position offsets in the alarm cards, the editor's bar buttons and the World Clock cards are named
  layout constants, and the city sheet's height bounds are named too. Sizes that depend on the window are unchanged.
- The app icon's white strokes use the system white token; the sheet dimming scrim is a named constant.
- Seconds and the hour-hand fraction use named time constants instead of inline literals.

## 1.2.2 — 2026-10-11

- Compliance pass: every visible control is published to the accessibility tree with its label and rectangle, including
  the stopwatch and timer buttons, alarm switches, Delete and Skip buttons, timer presets, rows and pickers, and the
  editor sheet's buttons, fields, pickers and pages.
- Interactive text and buttons use the system tint (LINK); the seconds hand keeps orange. Stop is neutral, and lap
  extremes and the error banner no longer use the destructive red.
- Cards, rows, the editor and the field use the Golden Gate radii (13 pt cards, 9 pt selection). Column headers use
  footnote size; the add-alarm and city-search fields use the 17 pt title size token.
- Keeps no unsaved state, so it is sudden-terminable from the first configure.
- Tests: `tests/compliance.rs` renders the root screen at 1200x780 and scale 1.5625 in Light and Dark.
- Still open: large titles on the tab roots, absolute y positions in the layout, and the shell's switch and text
  field widgets (the SDK has no switch yet).

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate window: a full-size window with the traffic lights in the unified toolbar; World Clock, Alarms, Stopwatch
  and Timers are a segmented control in the toolbar (the bottom tab bar is gone), with Edit and an Add (+) button.
- Follows Light and Dark: backgrounds, cards, text and hairlines use the system colour pairs.
- Toolbar buttons are published as controls, so the window does not drag from them. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 43 padding and gap values moved to the nearest standard step (at most 2 pt).
- 1 colour now uses the system colour token (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Clock for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Route physical text, Backspace and Return keys to existing field editors.
