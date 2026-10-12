# Changelog

## 1.6.0 — 2026-10-12

- Statistics on the paper tape: Copy Tape now ends with a Statistics block after the calculations. It gives the
  count, the sum, the mean, the standard deviation (from two calculations on), and the smallest and largest result.
- The statistics are worked out by the system's summary model, so a result that is not a number is left out.
- Numbers in the statistics follow the calculator's language, the same as the results above them.

## 1.5.0 — 2026-10-12

- Paper tape: the calculations in the history sidebar can be copied as a tape. Copy Tape sits at the left of the
  bottom row of the sidebar, next to Clear History. The tape lists the calculations oldest first, each with its
  result on the line below, right-aligned like an adding machine's printout.
- The tape is plain text on the shared clipboard, so it pastes into any app. Numbers follow the calculator's
  language (a decimal comma when Icelandic is chosen).
- Copy Tape is greyed out when the history is empty, and is published as a named control for accessibility.

## 1.4.0 — 2026-10-12

- New Convert mode: a Convert segment in the toolbar, next to Basic and Scientific. Pick a category (Length, Mass,
  Temperature, Speed or Volume), then the From and To units. Type the amount on the keypad; the converted value
  shows on the display, and Swap exchanges the two units.
- Length, mass and temperature are converted by the system's measurement model. Speed and volume use fixed factors.
- Numbers in Convert use the calculator's language: grouping and decimal separators follow the Decimal button.
- Copy and Paste work in Convert: Copy takes the converted value, Paste sets the amount.
- Convert is not kept across a restart; the calculator opens as before.
- Every Convert control is published as a named control for accessibility.
- Copy now goes through the system pasteboard service: one copy holds the number as plain digits and as the same
  number in the calculator's language, so other apps see the copy as a change on the shared clipboard.
- If the shell cannot take the copy, Copy still puts the number on the plain clipboard.

## 1.3.2 — 2026-10-11

- Edit menu: the app no longer adds its own Edit menu (it duplicated the shell's). Copy and Paste are offered to the
  shell's Edit menu through `commands`, enabled only when there is a number to copy or paste. Clear stays on the keypad
  and on Escape.
- Accessibility: the keypad, Delete, the display, the history rows, Clear History and the touch menu's Copy and Paste
  are published as named controls, so every control on screen has a label.
- Clear History uses the destructive colour. The selected operator follows Light and Dark.
- Radii come from the prefs layout (selection radius) and the key capsules (half the shorter side); key label sizes are
  named ratios.
- New `tests/compliance.rs`: the root screen at 1200 x 780 and scale 1.5625 in Light and Dark, its chrome, and labels.

## 1.3.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.3.0 — 2026-10-10

- Edit menu in the menu bar: Copy (⌘C), Paste (⌘V), Clear.
- App Intent: Evaluate (expression), answering with the result.

## 1.2.0 — 2026-10-10

- Golden Gate window: full-size, with the traffic lights in a unified toolbar. The toolbar has the history sidebar
  button, a Basic / Scientific segmented control and the decimal separator button; the "Calculator" title row is gone.
- History opens as a full-height sidebar beside the keypad instead of a centred sheet.
- Follows Light and Dark: window, keys, text and history use the system colour pairs; operators stay orange.
- Toolbar buttons are published as controls, so the window does not drag from them. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 5 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Calculator for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Support physical digits, decimal separators, operators, Return, Backspace and Escape.
- Draw scientific exponents using supported SF UI glyphs instead of missing modifier letters.
