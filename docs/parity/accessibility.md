# Accessibility (parity round 75, slice `accessibility`)

Scope: screen reader speech, Zoom, Increase Contrast, Reduce Transparency, Reduce Motion, cursor size and the
Accessibility Shortcut. Matrix source: `docs/parity-matrix/data/accessibility.json` (rows are stale; not edited here).

Reference evidence: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Library/CoreServices/UniversalAccessControl.app`
(the Accessibility Shortcut panel) but its Info.plist and `.strings` files are not in `extract/ref27/` yet, so no panel
labels or option lists were taken from the reference. The panel option set used here is the BifrOSt one (below). No REA
evidence ids were recorded this round; re-check when the UniversalAccessControl resources land.

## Rows

| Feature (matrix row) | API / behaviour | BifrOSt path | State |
|---|---|---|---|
| Screen reader bridge, announcements | `announce`, reader moves | `springboard/src/shell/accessibility/banner.rs` (`announce_tick`), `speech.rs` | Done: announcements are spoken while Screen Reader is on, through `espeak-ng` or `espeak` when on PATH. Silent without an engine. Needs Midget check (engine installed, audible output, a move interrupts the previous utterance). |
| Accessibility Shortcut (Option+Cmd+F5) | one key toggles chosen features | `springboard/src/shell/accessibility/shortcut.rs`, `hwkeys.rs` (`Shortcut::AccessibilityShortcut`, `KEY_F5`), `settings.rs` (`set_accessibility`) | Done for the key and the toggle; the choice is the `a11y_shortcut` pref (comma list of `screen_reader`, `zoom`, `reduce_motion`, `increase_contrast`, `reduce_transparency`, `button_shapes`, `dim_flashing_lights`). Not done: the panel and the Settings choice list. With nothing chosen the key does nothing. |
| Increase Contrast, Reduce Transparency, Reduce Motion (live) | toggles apply without restart | `accessibility/mod.rs` `publish()` runs on every shell tick (`shell/mod.rs` ~1274) | Already live through the appearance globals and the display options file; no change this round. Needs Midget check. |
| Zoom (full screen and picture-in-picture) | screen-wide magnification | not built | Not done. The only zoom is the Display Zoom geometry factor (`ZOOMS`) and the camera Magnifier. A screen magnifier needs a final-frame transform in the GPU presenter (`springboard/src/gpu/wl.rs`) and a lens rectangle for picture-in-picture. |
| Cursor size | pointer scale setting | not built | Not done. The hardware cursor is set from `bare/kms.rs` `set_cursor(argb)`; a size setting needs a scaled cursor image there and a value in the display options file. |

## Migrations and deletions

None. The speech path is new; the reader's announcements already went through `Banner::post`, which is unchanged.
`Banner::tick` now returns whether a new text came on screen (its only caller is `announce_tick`).

## Files touched

- `springboard/src/shell/accessibility/speech.rs` (new)
- `springboard/src/shell/accessibility/shortcut.rs` (new)
- `springboard/src/shell/accessibility/banner.rs` (speak on new announcement, screen reader gate)
- `springboard/src/shell/accessibility/mod.rs` (`pub mod shortcut;`, `pub mod speech;`)
- `springboard/src/shell/mod.rs` (fields `speech`, initialiser; shared file, a few lines)
- `springboard/src/shell/hwkeys.rs` (variant, key constant, chord, dispatch, one test line)
- `springboard/src/shell/settings.rs` (`set_accessibility` next to `accessibility()`)

## Midget checks still needed

- Install `espeak-ng` on Midget and confirm the reader's moves are spoken and a new move cuts the old one off.
- Option+Cmd+F5 with a chosen feature set (edit `a11y_shortcut` in settings.ini) toggles the Settings switches while Settings is open.
- Live toggles of Increase Contrast, Reduce Transparency and Reduce Motion repaint without a restart.
