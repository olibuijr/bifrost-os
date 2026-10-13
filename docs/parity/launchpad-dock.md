# Launchpad and Dock (slice `launchpad-dock`, round 75)

Scope: Dock recent apps, folder stacks, drag to and from the Dock, auto-hide and magnification, the Apps grid.

## What changed

- Downloads stack (new). `springboard/src/shell/dock_stack.rs`: a Dock entry (`DOCK_STACK`) between the minimized
  windows and the Trash. It draws a blue folder with the three newest visible files of `~/Downloads` fanned behind
  its flap. The folder is listed every 10 s from `Shell::dock_tick` (at most 2048 entries read), never per frame.
  A click opens the folder in Files.
- Magnification in Settings is a set, not a toggle (`Change::Magnification(bool)`), so the Settings switch and the
  dock's `desktop-dock.ini` value cannot drift apart.
- The Trash and the stack share `dock_stack::home_dir()` (the installed account home, else `$HOME`, else `/root`).

## Already in the code (checked, not changed)

- Recent apps section: `desk_dock` sections pinned | recents | minimized windows and Trash (`dock_section`).
- Auto-hide: `Change::Autohide`, slide animation, Option+Cmd+D (`dock_toggle_autohide`).
- Magnification: `magnified_rects` with a cosine falloff, hit-testing follows the live icons.
- Apps grid: paged, search, drag to reorder, page dots (`appgrid.rs`).

## Not done (still open)

- Stack fan/grid popover: the stack shows the fan as its icon only. A click opens Files, not a grid of the files.
- Apps grid folders: `appgrid.rs` has no folders; the home screen's `folders.rs` is not shared with it.
- Dragging an app from the Apps grid onto the Dock is not wired (the grid reorders only).

## Evidence

- No macOS reference files were read this round; the work follows the existing shell code and CHANGELOG entries.
- Midget check still needed: the stack icon at 48 and 80 pt, Downloads with 0, 1 and 3+ files, and a click on it.
