# parity(zoom-cursor): Screen Zoom, Cursor Size, Accessibility Shortcut chooser

Round 75. Code and unit tests only; not built or run by the agent (the lead builds). Nothing here is `parity` until
it works on Midget.

## Evidence

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` was searched for the Accessibility settings extension and
  zoom or cursor labels. The extracted string tables for the Accessibility settings were not present in
  `ref27/` at the time of this run, so no REA evidence ids were taken. Behaviour follows the shell's existing
  Accessibility page and the Option+Cmd shortcut scheme already used by the shell (Option+Cmd+D dock, Option+Cmd+F5
  Accessibility Shortcut). Re-check the labels once the lead extracts the settings extension.
- Existing code used as the base: `src/shell/accessibility/` (settings, `shortcut.rs` toggle and prefs),
  `src/shell/hwkeys.rs` (shortcut table), `src/bare/kms.rs` (hardware cursor plane), `src/gpu/wl.rs` (presenter).

## Rows

### Full Screen Zoom (Option+Cmd+8 and the zoom steps)

- Model: `springboard/src/shell/accessibility/zoom.rs`, `ScreenZoom` (`on`, `factor` 1x..8x, `mode`).
  `toggle()`, `step(bigger)` (x1.25 per step; zooming in turns zoom on, zooming out to 1x turns it off),
  `source_rect(pointer, screen)` returns the screen rectangle to sample, centred on the pointer and kept on the
  screen. `uv(rect, screen)` gives the normalised texture rect.
- Shortcuts: `springboard/src/shell/hwkeys.rs`. Option+Cmd+8 is `Shortcut::ZoomToggle`; Option+Cmd+= and
  Option+Cmd+- are `Shortcut::ZoomStep(true | false)`. Both dispatch in the `Shell` shortcut handler and update
  `Shell::screen_zoom` (`springboard/src/shell/mod.rs`). Locked screens ignore them.
- Done: the model, the shortcuts and the shell state, with unit tests in `zoom.rs`.
- Done (round 75): the shell passes `Shell::screen_zoom()` and the pointer to the Wayland GPU presenter every
  frame (`shell.rs` calls `Display::wayland_set_zoom` before both the composed present and the cursor-only present,
  and `Gpu::set_zoom` stores them in `springboard/src/bare/wayland.rs`). `Gpu::present` takes the whole screen as
  damage while zoom is on, and on the frame it turns off. After the composition draws the drawable,
  `Presenter::magnify` (`springboard/src/gpu/wl.rs`) copies the composed frame aside and draws the Full Screen
  magnification over the drawable. `magnify_for` in `bare/wayland.rs` turns `source_rect` and `lens_rect` into the
  presenter's normalised coordinates. The former `zoom_into` had no caller and is replaced by `magnify`.
  Option+Cmd+8 and the zoom steps request a full redraw (`hwkeys.rs`).
- Limit: zoom is drawn only by the Wayland GPU presenter. The CPU (no presenter) path and the bare scanout path
  (`bare/gpu_comp.rs`, used when the shell owns the panel) do not magnify.
- Midget check: Option+Cmd+8 turns zoom on; Option+Cmd+= and - change the factor; the view follows the pointer.

### Picture in Picture Zoom (lens)

- Model: `ScreenZoom::lens_rect(pointer, screen)` places a lens (360 x 240 px at 1x) below and to the right of the
  pointer, flipping to the other side at the screen edges. `Mode::Lens` makes `source_rect` sample a lens-sized area
  at the same factor. The Settings choice between Full Screen and Lens is not built: `mode` is `Mode::Full` by default.
- Done (round 75): the lens drawing. `Presenter::magnify` with `Magnify::lens` draws the magnified rect into
  `lens_rect` with a 2 px light rim (`zoom_lens`), over the unmagnified composed frame.
- Still open: a Settings control for the mode (`mode` stays `Mode::Full`), and the lens size at the shell's `ui.pt`
  scale (the lens is 360 x 240 screen px at 1x).
- Midget check: the lens follows the pointer and stays on the screen.

### Cursor Size (Settings slider; hardware plane)

- Setting: `AccessibilitySettings::cursor_size` (0..1), pref `a11y_cursor_size`, `cursor_scale()` = 1x..2x.
  Settings > Accessibility > POINTER shows the slider (`Ends::Cursor`, `Slider::CursorSize` in
  `springboard/src/shell/pages.rs` and `settings.rs`). The slider saves on release, like Liquid Glass.
- Plane: `springboard/src/bare/kms.rs`. `set_cursor(argb, scale)` now takes the scale. `cursor_side(scale)` gives the
  plane side (64 px at 1x, up to 128 px at 2x); `scale_sprite(src, side)` resamples the 64 x 64 sprite (nearest
  neighbour, hotspot at the top left). A driver that refuses the larger plane fails the ioctl, and `set_cursor`
  returns false so the caller draws the pointer in software.
- Done: the setting, the slider, the sprite scaling and the plane size, with unit tests in `kms.rs`.
- Done (round 75): the pointer grows in the shell's own drawing, on every backend. `gfx::cursor::Cursor::set_scale`
  (`springboard/render/src/gfx/cursor.rs`) scales the disc (12 px at 1x, up to 24 px at 2x) and forgets the shown spot
  so the panel is redrawn. The shell passes `Shell::cursor_scale()` (the open Settings slider while it is open, else
  the saved preference) each draw (`bin/shell.rs`). Unit test: `cursor_size_grows_the_disc_and_a_new_size_forgets_the_spot`.
- Not wired: `kms::Display::set_cursor(argb, scale)` still has no caller in the shell. Wiring it means the hardware
  plane takes over the whole pointer state (motion through `move_cursor`, the 3 s idle hide, screen off, lock and
  rotation, which hides it with a null handle). Done in the shell's frame loop, it would also double the pointer
  while the software disc is still stamped. Not done in this round because it needs Midget to verify. The
  Wayland path also has no hardware plane: `set_cursor` would issue DRM ioctls on the Wayland socket.
- Midget check: the pointer grows from 1x to 2x as the slider moves, and the disc stays on the pointer.

### Accessibility Shortcut chooser (Option+Cmd+F5)

- Toggle that already existed on the base: `springboard/src/shell/accessibility/shortcut.rs` (`Feature`, `parse`,
  `encode`, `toggle`, `Shell::accessibility_shortcut`) and the `a11y_shortcut` pref, a comma list of feature keys.
- Added here: the chooser panel. `AccessibilitySettings::shortcut: Vec<Feature>` is loaded from and saved to
  `shortcut::PREF`, in panel order. Settings > Accessibility > SHORTCUT > Accessibility Shortcut opens
  `PageId::AccessibilityShortcut`, a checked list of the seven features (`Pick::AccessibilityShortcut(index)`).
  Tapping a feature adds or removes it. The row's value is `pages::shortcut_label` ("None", the feature's name, or
  "N Features").
- The Settings window's copy is updated by the same save, so an open window shows the choice at once.
- Midget check: choose two features, press Option+Cmd+F5 (both switch on), press it again (both switch off).

## Files touched

- `springboard/src/shell/accessibility/zoom.rs` (new)
- `springboard/src/shell/accessibility/mod.rs` (`pub mod zoom`, `cursor_size`, `shortcut` field, prefs, tests)
- `springboard/src/shell/hwkeys.rs` (zoom key constants, `ZoomToggle`, `ZoomStep`, shortcut table, dispatch)
- `springboard/src/shell/mod.rs` (`screen_zoom` field and its default)
- `springboard/src/shell/list.rs` (`Ends::Cursor` and its end labels)
- `springboard/src/shell/pages.rs` (`Slider::CursorSize`, `Pick::AccessibilityShortcut`, `PageId::AccessibilityShortcut`, the Accessibility page rows, `shortcut_label`)
- `springboard/src/shell/settings.rs` (slider arm, pick arm, preview list)
- `springboard/src/bare/kms.rs` (`set_cursor(argb, scale)`, `cursor_side`, `scale_sprite`, tests)
- `springboard/src/gpu/wl.rs` (`Magnify`, `Presenter::magnify`, `zoom_state`/`zoom_pass`/`zoom_lens`; replaces `zoom_into`)
- Round 75 additions: `springboard/src/bare/wayland.rs` (`Gpu` zoom state, `Display::set_zoom`, `magnify_for`, present
  wiring, tests in `zoom_tests`), `springboard/src/bare/kms.rs` (`wayland_set_zoom` passthrough),
  `springboard/src/shell/mod.rs` (`screen_zoom()`, `cursor_scale()`), `springboard/src/shell/hwkeys.rs` (redraw on zoom
  keys), `springboard/src/bin/shell.rs` (zoom and cursor-size calls in the frame loop),
  `springboard/render/src/gfx/cursor.rs` (`set_scale`, scaled disc, test)

## Shared-file edits the lead must merge with care

- `springboard/src/shell/hwkeys.rs`: the `Shortcut` enum, the `out()` no-op list and the dispatch match. Other
  shortcut work touches the same lines.
- `springboard/src/shell/pages.rs` and `settings.rs`: enum variants and match arms, next to Accessibility rows.
- `springboard/src/shell/mod.rs`: one struct field and one initialiser.
- `springboard/src/bare/kms.rs`: the `set_cursor` signature changed; any caller added on another branch must pass the scale.

## Notes

- The hardware cursor and the zoom pass are model and hook only. Both need the frame and pointer owners to call
  them. Those calls are the next step, not part of this round.
- No matrix rows were edited (`docs/parity-matrix/` is the lead's).
