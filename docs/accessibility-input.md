# Accessibility input: dwell, slow keys, mouse keys, switch scan, voice control, earcons

Code: `springboard/src/shell/accessibility/` (`dwell.rs`, `switch_scan.rs`, `voice_control.rs`, `earcons.rs`),
`springboard/src/shell/slow_keys.rs`, `springboard/src/shell/mouse_keys.rs`, `springboard/src/shell/earcon.rs`, and the
input loop in `springboard/src/bin/shell.rs`. Settings pages are in `springboard/src/shell/pages.rs`.

## Built and wired

- **Dwell Control** (Settings > Accessibility > Touch). `DwellClick` is a pure model: a pointer that stays within
  `DEFAULT_RADIUS` of its anchor for the chosen delay produces one click. Moving farther starts a new dwell, and one dwell
  clicks at most once. The setting `a11y.dwell` is an index into `dwell::DELAYS_MS`, where 0 is off. The input loop in
  `bin/shell.rs` feeds it pointer samples and clicks at the resting position.
- **Slow Keys** (Settings > Accessibility > Keyboard). A key counts only when it is held for the delay. The setting
  `a11y.slow` is an index into `slow_keys::DELAYS_MS`, 0 is off. The input loop reads the delay with
  `accessibility::slow_delay_ms()` and passes key events through `SlowKeys`.
- **Mouse Keys** (Settings > Accessibility > Keyboard, `a11y.mouse_keys`). The keypad moves the pointer and the 5 key
  clicks. The keys are swallowed so they do not type. A held movement key starts slowly and reaches top speed after
  `RAMP_MS`; diagonal steps are scaled so they are not faster than straight ones. While Mouse Keys is on, the input loop
  draws a cursor even with no pointing device attached.
- **Earcons.** `accessibility/earcons.rs` synthesises short cues (focus, boundary, wrap, group, selection, error, mode,
  page turn) from notes; the repository ships no recordings. The screen reader queues them and `Shell::take_reader_earcons`
  hands them to the sound player. Popup cues (`shell/earcon.rs`: a menu appearing or closing) play only while the screen
  reader is on, so a sighted user hears no beeps for menus.

## Modelled, not wired

- **Switch Control scanning** (`accessibility/switch_scan.rs`). `Scan` has `next` (moves and wraps), `select` and an
  empty-scan rule. No code outside the module calls it, so no switch drives the desktop yet. The `Scanner` in
  `shell/storage.rs` is unrelated (storage page). A separate switch-scan model for the UI crate is in
  `springboard/ui/src/switch_scan.rs` (see `docs/parity/accessibility.md`).
## Voice Control (wired)

Settings > Accessibility > Voice Control (the `voice_control` preference) starts the shell's own recognition
(`services/recognizer.rs`: on device, the Dictation model and its consent, never an app's microphone). Every final
transcript goes through `Shell::voice_press` (`accessibility/voice_control.rs`); a command that cannot run is shown on the
announcement banner.

- "press X", "click X", "tap X" press the element called X (the element map, as `ax press` does). "run X" runs the app
  shortcut called X.
- "show numbers", "show names" and "show grid" label the front window (`accessibility/voice_overlay.rs`, drawn at the end
  of `Shell::draw` like the screen reader's ring). The targets are the screen reader's walk (`Shell::reader_walk`): the
  front window's Settings sidebar and page rows or an app's declared elements, then the menu bar titles, or only the open
  menu's rows. Numbers label each target 1, 2, 3 ...; names write the target's name on it.
- With numbers shown "press 3" presses the third label; with numbers or names shown "press Save" presses the visible
  element called Save (a name two labels share is refused: say a number).
- The grid cuts the front window into 3 by 3 numbered cells. "press 5" narrows the grid to cell 5; a cell no larger than
  48 pt is clicked at its centre instead, and "click here" clicks the centre of the grid as it stands. The click is a
  pointer press and release delivered by the next shell tick.
- "hide numbers", "hide names", "hide grid" remove the overlay. One overlay is shown at a time, and switching Voice Control
  off drops it.
- Not done: dictation into the focused field ("dictate ..."), scrolling and key commands from `ui::voice_commands`, and
  numbering of the Dock and of built-in apps other than Settings (the reader does not walk them yet).

## Modelled, not wired

- **Switch Control scanning** (`accessibility/switch_scan.rs`). `Scan` has `next` (moves and wraps), `select` and an
  empty-scan rule. No code outside the module calls it, so no switch drives the desktop yet. The `Scanner` in
  `shell/storage.rs` is unrelated (storage page). A separate switch-scan model for the UI crate is in
  `springboard/ui/src/switch_scan.rs` (see `docs/parity/accessibility.md`).

## Gaps

- Speak Items Under the Pointer is not built: the shell does not track hover over text (`accessibility/spoken.rs`).
- Braille output (`accessibility/braille_out.rs`) builds the line and the cells, but no display driver is attached.
