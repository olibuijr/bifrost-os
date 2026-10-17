# Picture in Picture

Code: `springboard/src/shell/pip.rs` (placement and automatic start), `services/picture_in_picture.rs` (the `pip`
service), `bifrost_ui::pip_window` (the life cycle and delegate events). The shell draws the window above the other
windows (`shell/mod.rs`, `pip::draw_window`).

## The pip service

- A player first says it can show a window: `possible {on}`. Without that, it gets no window.
- `start` asks for the window and `stop` takes it down.
- Each change sends the delegate events back to that player on the next tick: `pip.will_start`, `pip.did_start`,
  `pip.will_stop` and `pip.did_stop`.

## Placement (`pip.rs`)

- The window sits in a corner of the usable screen, which excludes the menu bar and the Dock. The rectangle is computed
  for the video's aspect ratio. The visible part is clipped to the surface, with fractions rounded outward.
- Settings > Desktop has the "Start PiP Automatically" switch (`pip_auto`). The switch only gates the automatic start.
  The player's own opt-in still applies.

## Gaps

Covering is not tracked, so the pip window is not hidden behind other windows by occlusion.
