# Changelog

## 1.6.8

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 1.6.7

- The Add Alarm sheet hangs centred from the toolbar; world map labels no longer overlap.

## 1.6.6

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 1.6.5

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 1.6.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 1.6.3

- World Clock faces draw the hands as the reference does: a short thick hour hand, a long minute hand that reaches the
  hour ticks and a thin second hand. The hour and minute hands were the same width and hard to tell apart.
- Map labels no longer print over each other: each city's name and time try the right of its dot, then the left,
  above and below, and a label with no free place is left out (London and Paris overlapped).

## 1.6.2

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## Unreleased

- World Clock shows a day and night map above the city cards: the land outline (Natural Earth 1:110m, public domain),
  the night side shaded behind the terminator computed from the sun's position, and each city as a dot with its name
  and local time. Each card adds the city's sunrise and sunset.

## 1.6.1 — 2026-10-24

- A timer preset sets the duration pickers; Start starts the timer.
- While a timer runs the right button pauses and resumes it; Add Timer in the toolbar sets up another one, so the
  picker no longer comes back over the running timer.

## 1.6.0 — 2026-10-23

- Alarm cards with the time, label and switch; delete with right-click or Delete.
- Running timers show as cards with progress; finished timers and alarms post a notification.
- One wheel notch moves a picker drum one row.

## 1.5.3 — 2026-10-20

- Nothing else changed: this release only rewords the notes for the new app icon.

## 1.5.2 — 2026-10-19

- The app has a new icon.

## 1.5.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.5.0 — 2026-10-13

- An alarm that fires while the Clock is closed now shows in Notification Centre under Clock. The shell filed these
  notifications under the app's package id, which matched no installed app, so they were dropped. They are now filed
  under the app's manifest id.
- The app's summary no longer says background alarms are unsupported on PC: the shell's Clock service rings them.
## 1.4.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

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
