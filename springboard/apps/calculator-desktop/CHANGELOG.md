# Changelog

## 1.13.2 — 2026-10-20

- Scientific: a function key (square root, cube root, logarithms, trigonometry) pressed right after a result now applies to the shown value instead of starting a new expression.
- Scientific: the cube-root key is drawn as a small 3 before the square root sign; it no longer shows a missing-glyph box.

## 1.13.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.13.0 — 2026-10-17

- Convert > Currency: ten currencies converted through the day's European Central Bank reference rates. The file is
  downloaded at most once a day on a background thread, kept in the app's data folder, and its date is shown under
  the units (`Updated <date>`). Offline, the last copy stays in use; with no copy, the result reads Unavailable.

## 1.12.0 — 2026-10-17

- Window > Always on Top: a checked item that keeps the calculator window above the other windows. The shell
  keeps the level while the app runs (`bifrost_app::windows::set_level`).

## 1.11.3 — 2026-10-14

- History uses the shared Golden Gate source-list rows; calculator geometry now names its display, menu and history
  dimensions and takes sidebar width from the shared layout token.

## 1.11.2 — 2026-10-14

- Derive the wide-keypad activation width from its key grid, and align menu assertions with the app-specific Calculator menu.

- The built-in app icon uses the shared named surface and label colours.

## 1.11.1 — 2026-10-14

- Place Copy Expression and Copy Last Result in the Calculator menu rather than duplicating the shell Edit menu.
## 1.11.0 — 2026-10-14

- Programmer readout: the value's ASCII code (0 to 127) follows the Unicode code point and the character, for example
  `U+0041  A  ASCII 65`.
- Parity matrix rows for the Programmer and RPN modes cite the readout and the stack line above the display, and state
  what is still missing (Show Binary, a deep stack that truncates).

## 1.10.0 — 2026-10-14

- Display settings have one model: thousands separators, decimal places and scientific notation are rendered by
  `format::Style`. The unused `options` module, a second copy of the same rules, is removed.
- Parity matrix rows for the Programmer and RPN modes, the display settings, the second-layer keys and the history
  list now cite the code that is wired in the UI.

## 1.9.0 — 2026-10-13

- Programmer mode (toolbar segment and View menu): an integer calculator on an 8, 16, 32 or 64-bit word in binary,
  octal, decimal (signed) or hexadecimal, with A to F, 00 and FF keys, AND, OR, XOR, NOR, NOT, two's complement,
  shifts by one bit and by an amount, rotates, byte and word flips, a clickable binary row and a Unicode readout.
  Hex digits and `& | ^ ~ < >` work from the keyboard.
- RPN mode (View > RPN Mode): the basic keypad drives a stack. Enter pushes, an operator takes the top two values, and
  the new Stack menu has Swap, Drop, Roll Up and Roll Down. The values under X show above the display.
- App menus: Edit gains Copy Expression and Copy Last Result; View has Basic, Scientific, Programmer, RPN Mode, Unit
  Conversion, Show History, Thousands Separator, Decimal Places (Automatic, 0 to 15), Scientific Notation and Word Size.
- Display settings: thousands separators can be switched off, results can use a fixed number of decimal places or
  scientific notation. They are saved with the other settings; an entry being typed is never rewritten.
- Second layer of the scientific keypad: y to the x (under e to the x), 2 to the x (under 10 to the x), log base y
  (under ln) and the x-th root of y (under the y-th root key). `logy` and `rootx` are infix words in the evaluator.
- History: the first calculation of each day carries a day caption, a swipe to the left or the right-click menu
  deletes one calculation (the menu also copies its result or expression), and Clear History needs a second tap.
- Keyboard entry on the scientific keypad: `( ) ^ ! e p`, Shift-E for the exponent and Option-minus for the sign.
- Convert: area, time, data size, energy, pressure, power, angle and fuel economy join the five categories; the
  category segments take two rows.

## 1.8.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.8.0 — 2026-10-12

- Warnings cleared: the test-only helpers (the hit-test rectangles used by the tests, `describe`, the converter's
  `describe`, the calculator's icon) are compiled for tests only, and the Canvas drawing no longer imports the CPU
  surface outside the tests.
- Engine recall removed: a history result is recalled through `recall_value`, which the Calculator already used.
- Scientific keypad: the second-function layer of the Memory Recall key (`mr`) now shows `ans`, which types the last
  result, the way the Answer key does on other calculators.

## 1.7.0 — 2026-10-12

- Drawing through Canvas (graphics wave G2, `app-canvas-pilot`): the calculator draws its display, keys, history
  sidebar, Convert panel, Edit menu and toolbar with the drawing vocabulary of `bifrost_render::canvas::Canvas`
  (`draws_canvas` is true, `draw_canvas` does the drawing). The same routine runs on the shared-memory CPU surface and
  on the GPU direct path, so the calculator's pixels do not depend on which one the window server uses.
- The toolbar and its band are drawn by the calculator itself on `Canvas` (the SDK helpers take a surface), with the
  same layout, glyphs and accessibility records. The translucent titlebar material of the toolbar band is not drawn
  yet: the band is opaque until the SDK toolbar takes a canvas.

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
