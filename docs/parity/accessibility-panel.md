# parity(accessibility-panel): Accessibility Shortcut panel, spoken-element focus ring, Settings search keywords

Round 75. Code and unit tests only. Not built or run by the agent (the lead builds). Nothing here is `parity` until it
works on Midget.

## Evidence

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists the Accessibility Shortcut panel's bundle, but its
  Info.plist and `.strings` files were not in `extract/ref27/` when this slice ran. The panel's labels and option list
  therefore follow the BifrOSt chooser (`shortcut::Feature`, seven features). No REA evidence ids were taken. Re-check
  the panel title and footer once the resources land.
- Base read: `docs/parity/accessibility.md` (Round 75 rows), `docs/parity/zoom-cursor.md` (chooser page), the shell's
  `src/shell/accessibility/reader.rs` (focus ring), `src/shell/character_viewer.rs` (the floating-panel pattern reused here).

## Rows

### Accessibility Shortcut panel (Option+Cmd+F5)

- Before: the key switched every chosen feature at once (`shortcut::toggle`), with nothing on screen.
- Now: the key opens `springboard/src/shell/accessibility/shortcut_panel.rs`, a floating panel listing the chosen
  features in panel order, each with a checkbox that shows the feature's live state. A tap on a row switches that one
  feature and saves the choice (pref, open Settings copy, live hooks). Done, Escape, or Option+Cmd+F5 again hides the
  panel. The panel takes every touch while it is up, as the character viewer does.
- With no feature chosen the panel still opens and says where to choose them. The old behaviour (the key does nothing)
  is gone; this is a deliberate change.
- Pure geometry (`panel`, `row_rect`, `done_rect`, `hit`) and the model (`ShortcutPanel::toggle`, `features`) are
  unit tested in `shortcut_panel.rs`.
- Wiring (shared-file edits, a few lines each): `shell/mod.rs` (field `a11y_panel`, initialiser, touch routing, modal draw
  and the damage gates), `shell/hwkeys.rs` (Escape hides the panel first), `shell/accessibility/mod.rs` (`pub mod shortcut_panel`).
- Settings > Accessibility > SHORTCUT footer and the FEATURES page footer now describe the panel (`shell/pages.rs`).
- Midget check: choose two features in Settings, press Option+Cmd+F5, the panel lists both with the right checkboxes;
  tap one, only that switch moves in Settings; Escape closes; Option+Cmd+F5 twice closes and reopens.

### Spoken-element focus ring

- Already built before this round: `shell/accessibility/reader.rs` `Shell::draw_reader` strokes a 2 pt ring around
  the element the screen reader's cursor is on, in the link colour, and the frame repaints under it (`Reader::busy`).
  `Shell::draw` calls `draw_reader` after the desk and banner (`shell/mod.rs`). Not changed in this round.
- Not done: the ring is a single stroke, not a thicker two-tone ring, and it is not drawn on built-in apps (their
  elements are not walked yet, see the module doc of `reader.rs`).
- Midget check: Settings > Accessibility > Screen Reader on, Ctrl+Alt+Right moves a visible ring from row to row.

### Settings Accessibility search keywords

- `shell/settings_search.rs` `KEYWORDS` table: root rows answer to words besides their title. The `Accessibility` row
  answers to screen reader, speech, zoom, magnifier, contrast, transparency, reduce motion, motion, shortcut, cursor,
  pointer, switch scanning, touch accommodations, flashing lights, button shapes and similar words.
- `filter` matches a row by title or by a keyword (`matches`, `keywords`). The search still narrows only the root page;
  the Accessibility sub-page rows are not searchable yet.
- Midget check: type "zoom" or "shortcut" in Settings search; the Accessibility row appears.

## Files touched

- `springboard/src/shell/accessibility/shortcut_panel.rs` (new: panel geometry, model, draw, Shell glue, tests)
- `springboard/src/shell/accessibility/shortcut.rs` (`accessibility_shortcut` opens or closes the panel; docs)
- `springboard/src/shell/accessibility/mod.rs` (`pub mod shortcut_panel;`)
- `springboard/src/shell/mod.rs` (shared file: field, initialiser, touch routing, draw and damage gates)
- `springboard/src/shell/hwkeys.rs` (Escape hides the panel; shared file, one line)
- `springboard/src/shell/pages.rs` (two footers)
- `springboard/src/shell/settings_search.rs` (`KEYWORDS`, `keywords`, `matches`, one test)

## Migrations and deletions

None deleted. `shortcut::toggle` stays: the panel's row tap calls it with one feature. `accessibility_shortcut` keeps its
name and signature; its body now opens the panel instead of switching every feature.

## Shared-file edits the lead must merge with care

- `springboard/src/shell/mod.rs`: one struct field, one initialiser, one touch-routing block next to the character
  viewer, the modal list and draw, and `self.a11y_panel.is_none()` added to the two quiet-frame checks (`Shell::draw`).
- `springboard/src/shell/hwkeys.rs`: one `Shortcut::Back` guard line above the character viewer's.

## Not done

- Panel title, footer and option list against the reference (see Evidence).
- The Accessibility sub-page rows are not searchable.
- The focus ring's shape and width against the reference.
- Nothing was built or run, per the wave rules. The modules are unverified by the compiler and tests.
