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
- **Voice Control** (`accessibility/voice_control.rs`). The parser accepts only final transcripts and reads "press", "click"
  or "tap" followed by an element name. `Shell::voice_press` resolves the name in the element map, the same way `ax press`
  does. Only tests call it: the recognizer session for Voice Control is not started, so nothing feeds the parser on a
  device. Speech recognition itself is described in `docs/speech-dictation.md`.

## Gaps

- Speak Items Under the Pointer is not built: the shell does not track hover over text (`accessibility/spoken.rs`).
- Braille output (`accessibility/braille_out.rs`) builds the line and the cells, but no display driver is attached.
