# Window management and Spaces (slice `windows-spaces`, round 75)

Scope: all windows and the desktop strip (overview), several desktops with switch keys, window tiling (drag to an
edge or corner, Fn+Ctrl+arrow layouts), the focus strip (Stage Manager), full-screen windows in their own space, and
the app overview (App Exposé). The feature names are not used in code or comments; the code uses neutral names:
`Spaces` (desktops and full-screen spaces), `tiling::Layout`, `FocusStrip`, `overview` and `app_overview`.

## Evidence

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Applications/Mission Control.app`, but its
  bundle is not yet in `~/Projects/macos-ref/extract/ref27/` (only Books, Calculator, Calendar, Contacts, Dictionary,
  FindMy, Font Book, Freeform, Mail, Maps, Notes, Photos, Preview, Reminders, Stickies, TextEdit, Utilities and Weather
  are extracted). No REA evidence was taken for this slice. Behaviour below comes from the existing window model
  (`springboard/wm/src/wm.rs`) and the design rules in `docs/bifrost-ui.md`, so every row needs a Midget check
  against the reference once the bundle lands.
- Existing code before this round: `wm/src/wm.rs` had Ctrl+Super half snapping (`Side`, `snap`), the centre key
  (`restore_centre`) and maximize. No caller of `snap` existed in the compositor (no key dispatch in `wm/src`), so the
  snap model was unwired too.

## What changed

| File | What |
|---|---|
| `wm/src/tiling.rs` (new) | `Layout` (Maximize, Left, Right, Top, Bottom, four quarters) and `frame(area, layout)`; `drop_zone` for a drag to an edge or corner (`EDGE` 8 px, `CORNER` 64 px; top = maximize, a drop above the menu bar still counts); `Arrow` and `for_arrow` (Left and Right tile to a half, Up maximizes, Down returns to own size). |
| `wm/src/wm.rs` | `Side` and `half` deleted; `Window.tiled` is `Option<Layout>`; `snap` renamed to `tile(id, layout)` (`Layout::Maximize` is the maximize state, so unmaximize still returns to the frame from before the first tile); new `Wm::drop_zone(x, y)`; `snapped` returns `Option<Layout>`. Tests migrated and extended. |
| `wm/src/spaces.rs` (new) | `Spaces`: desktops and full-screen spaces in switch order, `step` (clamped at the ends), `desktop_number` (Ctrl+1..9), `add_desktop`, `remove_desktop` (windows and full-screen homes move to the nearest desktop), `place`, `move_to`, `enter_full_screen` (own space after the desktops, shown), `exit_full_screen` (back to the home desktop), `forget` (window closed). |
| `wm/src/stage.rs` (new) | `FocusStrip`: most recent window in the large frame (`touch`, `current`), thumbnails down the left edge (`thumbnails`, 168x105, 12 px gaps, extras left out when they do not fit), `stage_frame` for the large frame. |
| `wm/src/overview.rs` (new) | `cells` (square-ish grid, last row centred), `fit` (aspect kept, centred in its cell), `desktop_strip` (desktop tiles along the top), `overview` (strip plus window grid), `app_overview` (one app's windows over the whole area). |
| `wm/src/lib.rs` | `pub mod overview; pub mod spaces; pub mod stage; pub mod tiling;` |

## Per row (no matrix rows matched these names; nothing closed in `docs/parity-matrix/`)

- **Mission Control (all windows, Spaces strip)**: model done in `overview::overview`. Not drawn. Needs the shell to
  draw the desktop tiles and thumbnails (a `Layer`-style snapshot of each window, if the compositor can give one) and
  to hit-test them. Midget check: the overview opens, shows every window, and a click on a desktop tile switches to it.
- **Spaces and switch shortcuts**: model done in `spaces::Spaces`. Not wired: no key dispatch in `wm/src`, and the
  shell has no desktop list. Next: have the Wm own a `Spaces`, map only `shown()` windows, and bind Ctrl+Left/Right and
  Ctrl+1..9. Midget check: switching hides and shows the right windows, and a new window opens on the desktop in view.
- **Window tiling (drag to edge, Fn+Ctrl arrows)**: the layouts, drop zones and arrow mapping are done and tested in
  `tiling.rs`; `Wm::tile` and `Wm::drop_zone` are in `wm.rs`. Not wired: the move grab (`Wm::motion` for `Grab::Move`)
  does not show a drop preview or call `tile` on release, and no arrow-key binding exists. Midget check: dragging to the
  top maximizes, to a side tiles to that half, and the frame from before the tile is restored on the next drag.
- **Stage Manager (focus strip)**: model done in `stage::FocusStrip`. Not wired: the shell does not feed `touch` on
  focus, and the large frame is not applied. Midget check: a window used from the strip comes to the large frame.
- **Full-screen apps in their own space**: `Spaces::enter_full_screen` and `exit_full_screen` are done and tested. Not
  wired to the xdg fullscreen request in `wm/src/handlers.rs` (no handler exists yet). Midget check: a full-screen app
  takes its own space, and leaving it returns to the desktop it came from.
- **App Exposé (one app's windows)**: `overview::app_overview` is done. Not wired (needs the app id per window, which
  the shell has in its window list). Midget check: the grid shows only the windows of the app that was invoked.

## Still a model, not behaviour

Nothing in this round changes what a user sees: the new modules are unit-tested pure models, and the existing
`Wm::tile` path has no key or drag caller. Wiring is the next step and touches `wm/src/handlers.rs`, `wm/src/grabs.rs`,
`wm/src/state.rs` and the shell's window list, which were not part of this slice's brief.

## Tests (run by the lead; not built here)

`cargo test -p bifrost-wm --lib` covers `tiling::tests`, `spaces::tests`, `stage::tests`, `overview::tests` and the
migrated `wm::tests` (`tiling_*`, `a_drag_to_an_edge_*`, `a_tiled_window_follows_a_new_usable_area`, `the_half_split_*`).
