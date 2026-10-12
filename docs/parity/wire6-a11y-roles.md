# Wire 6: screen reader roles, rotors and activation

Branch `wire6/a11y-roles`, from `main` (release 0.5.43). Extends the screen reader cursor from wave 5 (`wire5-screen-reader.md`):
the cursor now walks the menu bar, the rows of an open menu and the front window's content (a Settings page or an app
window's declared elements), speaks each element as `kind, name, value` (`button, Close`), rotates through rotors, and presses
the element under the cursor. No protocol change: the app elements come from the existing `controls` and `controls_tree`
the service channel already carries, and presses go through `ax_press`. No builds were run in this round (the lead builds
once). The host unit tests were written by hand with expected values worked out from the code; they have not been run.

## What is now wired

1. **Element roles and speech.** `springboard/src/shell/accessibility/walk.rs`.
   - `speech` says the kind first, then the name, the value and `unavailable`: `button, Close`, `switch, Bold Text, on`,
     `menu, File`, `menu item, Save, unavailable`, `Message, Hello` (a static text row has no kind word). This changes the
     wave 5 order (`Bold Text, switch, on` is now `switch, Bold Text, on`).
   - `spoken_kind` maps the role words the map uses (`page`, `toggle`, `slider`, `choice`, `field`, `segmented-control`,
     `menu`, `item`, `menu-item`, `window`, `app`, `progress-indicator`, and the rest of `ax::role_name`) to words.
2. **The walk.** `Shell::reader_walk` in `springboard/src/shell/accessibility/reader.rs`, in this order:
   - the menu bar titles (`menu_buttons`), each a `menu` element with its bounds in the bar;
   - the rows of the open menu (`desktop_menu`, `menu_rows`, `menu_spans`), each a `menu-item`, separators skipped;
   - the front window: a Settings page (`settings_page`, as in wave 5, now offset to screen pixels), or an app window
     (`app_walk`: `add_app_tree` under `app/<id>`, bounds scaled by `ui.pt(1.0)` and offset by the window body).
3. **Rotors.** `walk::rotor_name`, `walk::in_rotor`, `Reader::step_in`, `Reader::cycle_rotor`. Rotor 0 is All; rotors 1 to 6
   are the shared `bifrost_ui::ax::STANDARD_ROTORS` (Controls, Fields, Lists, Menu items, Text, Progress). Settings `page` rows
   match Controls as buttons, `choice` rows match Lists, and menu titles and rows match Menu items (`walk::rotor_role`).
   The new rotor's name is spoken when it is picked.
4. **Activation.** `Shell::reader_activate` presses the element under the cursor:
   - a menu title opens its menu (a press on the title, through `desktop_bar_event`);
   - a menu row runs its action (press and release on the row, through `desktop_bar_event`);
   - a Settings row is tapped at its centre (`Settings::touch`, down then up), so a switch flips and a page opens;
   - an app element is pressed through `Shell::ax_press` with its address, the same tap the `ax press` command sends.
   A disabled element is spoken with `unavailable` and not pressed. A press that fails (for example, the app is gone) is
   spoken as its error.
5. **Keys.** `hwkeys.rs`: `Shortcut::ReaderRotor(bool)` on Ctrl+Alt+Down (next) and Ctrl+Alt+Up (previous);
   `Shortcut::ReaderActivate` on Ctrl+Alt+Space. Both are handled only while the screen reader is on. Ctrl+Space (input
   source) and the Meta chords are unchanged. The existing Ctrl+Alt+Right and Left step the current rotor now.
6. **Focus ring.** `draw_reader` draws the ring at the cursor's element in screen pixels (the walk gives absolute bounds), so
   it works on the menu bar and on app windows as well as on Settings.

## How a user reaches it

1. Settings > Accessibility > VISION > `Screen Reader` on.
2. Ctrl+Alt+Right walks everything in the All rotor: the first menu title, then the rest of the bar, then the front window.
   The banner speaks each one, for example `menu, File`.
3. Ctrl+Alt+Space on `menu, File` opens the menu. Ctrl+Alt+Right moves into its rows (`menu item, Save`), and Ctrl+Alt+Space
   runs the row.
4. Ctrl+Alt+Down picks the Controls rotor and speaks `Controls`; Ctrl+Alt+Right then walks only buttons, switches and sliders.
5. With Settings in front, Ctrl+Alt+Space on `switch, Bold Text, off` flips it; on `button, General` it opens the page.
6. With an app in front that declares its controls, Ctrl+Alt+Right walks its elements and Ctrl+Alt+Space presses one.

## How to verify on Midget (test PC)

1. `bifrost pc midget tap-window Settings X Y` on Accessibility, then on `Screen Reader` under VISION (`a11y_screen_reader=1`).
2. `bifrost pc midget key ctrl+alt+right` (or the lab's chord path). The first ring is on the first menu title and the banner
   reads `menu, <title>`. Walk the bar and check each title is spoken once.
3. `bifrost pc midget key ctrl+alt+space` on a title: its menu opens. Walk to a row and press the chord again: the action runs.
4. `bifrost pc midget key ctrl+alt+down`: the banner says `Controls`. Walk: only buttons, switches and sliders get the ring.
5. Open Settings > General. Walk to `button, Software Update`, press the chord: the page opens. Walk to a switch and press
   the chord: it flips and the banner reads the new value.
6. Open an app that declares its controls (`bifrost pc midget ax list` shows its `app/` elements). Walk and press one.
7. Turn the switch off: the ring disappears on the next frame and the chords do nothing.

## REA evidence

- Accessibility collection (`https://developer.apple.com/documentation/accessibility`):
  `ev_2cd6d5dc5ba116291b186263ebf1503997a1ba87641ccbcf7cf3c9e0f5ec8ceb` (recorded in wire rounds 2 to 5).
- Accessibility Inspector (`https://developer.apple.com/documentation/accessibility/accessibility-inspector`):
  `ev_7b0cc149a8095e216a8f8e3cab7670f084472241ade8b122ebda267bd0b78e35`.
- Matrix evidence on the accessibility row: `ev_2a505504e4634c45494730263b3e9dd7da9a8a53b707a6c6c2a02409b4820b58`.
- No new REA capture was taken in this round: the build-free pass did not start the browser tooling. The role words and the
  rotor sets follow the model in `springboard/ui/src/ax.rs` (`role_name`, `STANDARD_ROTORS`) and the existing notes.

## Gaps (not wired in this round)

- **No heading or link rotor.** `bifrost_ui::ax::Role` has no heading or link role, so the tree does not carry them. Add the
  roles to the model first, then a rotor entry in `STANDARD_ROTORS` picks them up.
- **The Dock is not walked.** Its icons move with magnification and the dock layout is not a list the reader can read; the
  `os/apps` elements exist in the map but have no bounds on screen.
- **Built-in apps other than Settings are not walked.** An app drawn by the shell has no element tree; its controls are only
  in the map when the app declares them (`controls`).
- **The cursor does not scroll.** A row outside the visible area of a Settings page can be reached but the ring sits off screen.
- **The cursor keeps its place only by address.** A rotor change to a rotor that does not hold the cursor's element starts from
  the first or last element of that rotor, not the nearest one.
- **Speech is the banner and the AT-SPI announcement only.** The shell's own announcements still do not reach the bridge as
  `Object:Announcement` (see `wire5-screen-reader.md`).
- **Not run on Midget.** The keys go through the hardware path in `hwkeys.rs`; the chords are host unit tests only.
- **Settings footer does not name the new keys yet.** `pages.rs` is a shared file; its footer under `Screen Reader` still names
  only the wave 5 keys.

## Matrix rows

The matrix data is `docs/parity-matrix/data/accessibility.json`. The lead updates the rows; this round does not edit
`docs/parity-matrix/`. The rows below stay `partial` until the reader runs on Midget.

| Matrix slug | Feature (as written in `docs/parity-matrix/data/accessibility.json`) | BifrOSt path |
|---|---|---|
| accessibility | Screen reader bridge (VoiceOver equivalent) | springboard/src/shell/accessibility/reader.rs (walk, rotors, activation, focus ring); springboard/src/shell/accessibility/walk.rs (speech, rotor sets, menu elements); springboard/src/shell/hwkeys.rs (ReaderRotor, ReaderActivate keys) |
| accessibility | Element role mapping | springboard/src/shell/accessibility/walk.rs (spoken_kind, rotor_role, in_rotor); springboard/src/shell/axmap.rs (item_role, shared with the cursor) |
| accessibility | Labels, titles and help text | springboard/src/shell/accessibility/walk.rs (speech: kind, name, value, unavailable); springboard/src/shell/accessibility/reader.rs (menu titles and rows, app elements with their labels) |
