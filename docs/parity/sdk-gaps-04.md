# Parity sdk-gaps-04: pane Help entries, scene hand-off, photo adjustment records

Three models in `springboard/ui/src` (crate `bifrost-ui`), each pure and tested headless in
`springboard/ui/tests/parity_sdk-gaps-04.rs`. Nothing here draws, opens a window or reads a clock.

## Help menu entries from a pane (`help_menu`)

Mirrors the way a settings pane adds its Help menu entries while it is selected (`updateHelpMenu(with:)` and the
title and anchor keys of a pane's Help menu information). `PaneHelpMenu` keeps the entries of each selected pane,
replaces them on update, moves the updated pane to the end of the menu order and removes them when the pane is
unselected. `HelpEntry` carries a trimmed title and an optional anchor into the pane's help.

- Wired: nothing yet. The shell's Help menu rows are fixed in `springboard/src/shell/desktop_bar.rs`.
- Needs: the shell calls `update` when a pane is selected and `remove` when it is unselected, draws the entries after
  the fixed rows, and opens the anchor from a help lookup that does not exist yet.

## Scene hand-off to an app-owned renderer (`scene_handoff`)

Mirrors the way a scene is handed to a renderer the app owns: the app calls the scene's update with the time, runs the
steps the scene returns, then encodes its own draw commands for a render pass. `SceneHandoff` wraps the existing
`scene_loop::SceneLoop`. A render pass names the viewport in device pixels (points times the scale, rounded) and the
scene time as of the last update. A pass is refused before the first update after a presentation, after a dismissal,
and when the scene has no area in device pixels.

- Wired: nothing yet. The scene loop it wraps has no caller outside this hand-off, and the GPU presenter
  (`springboard/src/gpu/`) does not take a render pass.
- Needs: an SDK entry point that gives an app its hand-off, and a presenter path that accepts an app's render pass.
  There is no second GPU pipeline; the presenter is Linux GPU code (`docs/gpu-first.md`).

## Adjustment records of edited photos (`photo_adjustment`)

Mirrors the adjustment data an editing extension stores with an edited photo: an editor format identifier, a format
version and the editor's payload. `EditSession` starts from the record the photo already has (only when the record
belongs to the same editor format), accepts a committed record only from that format, and reverts the photo to its
original. `result` is the record the photo keeps once the session ends.

- Wired: nothing yet. The built-in editor (`springboard/src/shell/photos/edit.rs`) keeps its own edit state.
- Needs: the editor writes and reads these records beside the original file, and an extension host that loads
  third-party editors into the Photos app.
