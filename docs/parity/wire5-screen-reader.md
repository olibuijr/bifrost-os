# Wire 5: Screen reader cursor in Settings

Branch `wire5/screen-reader`, from `main` (release 0.5.42). Wires a keyboard-driven screen reader cursor over the visible
Settings page: a Settings switch, next and previous element keys, a focus ring, and speech through the announcement queue.
No builds were run in this round (the lead builds once). No protocol change; no service added. Host unit tests were written
by hand with expected values worked out from the code; they have not been run.

## What is now wired

1. **The setting.** `AccessibilitySettings::screen_reader` (`springboard/src/shell/accessibility/mod.rs`), stored as
   `a11y_screen_reader` (`1` or `0`, default off). Loaded and saved with the other accessibility keys; the round-trip test
   sets it on.
2. **The Settings switch.** Settings > Accessibility > VISION > `Screen Reader`, the last row of that group, with a footer
   naming the keys (`springboard/src/shell/pages.rs`, `Toggle::ScreenReader`; the handler is in `springboard/src/shell/settings.rs`
   next to the other accessibility toggles).
3. **The cursor model.** `springboard/src/shell/accessibility/reader.rs`.
   - `Reader::step(els, forward)` moves the cursor over the enabled rows of a page, wrapping at both ends. With no cursor,
     forward starts at the first row and backward at the last. The cursor is kept by address, so it survives a page rebuild.
   - `speech(element)` says the name, the kind and the value: `Bold Text, switch, on`, `Display Zoom, button, Standard`,
     `Wi-Fi, button, unavailable` for a disabled row.
   - `page_elements` turns a Settings page into `bifrost_ui::ax::Element`s with the same roles and values as the element map
     (`item_role` in `springboard/src/shell/axmap.rs`, now shared by the map and the cursor), each with its row rectangle.
4. **Keys.** `Shortcut::ReaderStep(bool)` in `springboard/src/shell/hwkeys.rs`: Ctrl+Alt+Right moves forward, Ctrl+Alt+Left
   back. Handled in `hw_shortcut` only while the switch is on. The arrow keys with Meta+Ctrl still tile windows, unchanged.
5. **Speech.** Each move posts the row's speech to the announcement queue (`Banner::post`, assertive priority). The banner
   shows it at the top of the screen for 4 s, the same path an app's `announce` takes.
6. **Focus ring.** `Shell::draw_reader` draws a 2 pt ring in `LINK` (systemBlue) around the cursor's row, offset by the
   front Settings window's content origin (window rectangle plus title height, none for full-size windows), with the
   selection radius from `prefs_layout::m::SELECTION_RADIUS`. The damage check (`reader_damage`) repaints the area when the
   cursor moves and when a ring is removed.

## How a user reaches it

1. Open Settings, go to Accessibility, and turn on `Screen Reader` under VISION.
2. Open a Settings page (for example General > Software Update). Press Ctrl+Alt+Right: the first enabled row gets a ring and
   the banner shows its name, kind and value. Keep pressing to walk the page; Ctrl+Alt+Left goes back; the cursor wraps.

## How to verify on Midget (test PC)

1. `bifrost pc midget tap-window Settings X Y` on the Accessibility row, then on `Screen Reader` under VISION. `settings.ini`
   gets `a11y_screen_reader=1`.
2. Open General > Software Update. `bifrost pc midget key ctrl+alt+right` (or the lab's key path for a chord). A ring appears
   around the first enabled row and a dark banner reads its name, for example `Automatic Updates, switch, on`.
3. Press the forward chord again: the ring moves to the next enabled row and the banner changes. A disabled row is skipped.
4. Press the backward chord from the first row: the ring goes to the last enabled row.
5. Turn the switch off: the ring disappears on the next frame and the chords do nothing.

## REA evidence

- Accessibility collection (`https://developer.apple.com/documentation/accessibility`): recorded in the earlier wire rounds,
  `ev_2cd6d5dc5ba116291b186263ebf1503997a1ba87641ccbcf7cf3c9e0f5ec8ceb`.
- Accessibility Inspector (`https://developer.apple.com/documentation/accessibility/accessibility-inspector`):
  `ev_7b0cc149a8095e216a8f8e3cab7670f084472241ade8b122ebda267bd0b78e35`.
- No new REA capture was taken in this round (the browser tooling was not started for a build-free wiring pass). The
  behaviour is taken from the matrix row's existing notes and the rotor model in `springboard/ui/src/ax.rs`.

## Gaps (not wired in this round)

- **Speech is the banner only.** The shell's own announcements do not reach the AT-SPI bridge: `atspi::bridge` emits
  `Object:Announcement` only for an app slot (`announce(slot, text, priority)`). Speaking the Settings cursor on a live
  screen reader needs a shell speech source on the bridge; the banner is the only on-screen and spoken path today.
- **Rotor is not wired.** `ax::STANDARD_ROTORS` and `Map::rotor_step` exist; the cursor walks every enabled row and does
  not filter by rotor (Controls, Fields, Lists...). Settings rows use roles `page`, `toggle`, `slider`, `choice`, which
  the standard rotor role names do not match yet.
- **Only Settings is walked.** App windows are not walked: an app draws its own surface, so a ring from the shell would sit
  under the app. The app's `ax` tree is still reachable by `bifrost pc midget ax list`. The sidebar of a split Settings
  window and the menu bar are not walked.
- **No scrolling.** The cursor does not scroll the page to a row outside the visible area, so the ring can sit off screen.
- **No activation.** Enter or Space does not press the row under the cursor yet; `ax press` still does that.
- **Not run on Midget.** The keys go through the hardware key path in `hwkeys.rs`; the chord is a host unit test only.

## Matrix rows

The matrix data file is `docs/parity-matrix/data/accessibility.json`. The lead updates the rows; this round does not edit
`docs/parity-matrix/`.

| Matrix slug | Feature (as written in `docs/parity-matrix/data/accessibility.json`) | BifrOSt path |
|---|---|---|
| accessibility | Screen reader bridge (VoiceOver equivalent) | springboard/src/shell/accessibility/reader.rs (cursor, speech, focus ring); springboard/src/shell/hwkeys.rs (ReaderStep keys); springboard/src/shell/pages.rs (Screen Reader row) |
| accessibility | Accessibility announcements | springboard/src/shell/accessibility/reader.rs (speech queued on each move, `Banner::post`); springboard/src/shell/accessibility/banner.rs |
| accessibility | Element hierarchy (parent, focus, checked state) | springboard/src/shell/accessibility/reader.rs (focus ring on the cursor's row); springboard/src/shell/axmap.rs (item_role, shared role mapping) |
