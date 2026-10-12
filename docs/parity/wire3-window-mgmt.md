# Wire 3: window management (desktop shell)

Wires the existing desk.rs window helpers (`Desk::tile`, `tile_rect`, `snap_zone`, `desk_zoom_window`,
`desk_cycle_window`) to the Window menu, the keyboard and the title-bar drag. No new model was written; the
window geometry comes from `springboard/src/shell/desk.rs` (`Tile`, `tile_rect`, `snap_zone`).

## What is now wired

| Feature | How a user reaches it | Code |
|---|---|---|
| Left Half, Right Half, Fill, Center | Window menu rows (enabled when a desktop window is front and no foreign window is active); Ctrl+Cmd+Left, Ctrl+Cmd+Right, Ctrl+Cmd+Up (Fill), Ctrl+Cmd+Down (Center) | `desk.rs` `desk_tile_front`; `desktop_bar.rs` `Action::Tile`; `hwkeys.rs` `Shortcut::Tile` |
| Return to Previous Size | Window menu, "Return to Previous Size" | `desk.rs` `desk_restore_front` → `Desk::return_to_previous` |
| Snap on drag | Drag a window by its title bar to a screen edge (half), the top edge (Fill) or a corner (quarter); release to tile | `desk.rs` `Event::Up` arm for `Grab::Move`, using `snap_zone` with an 8 pt band |
| Next Window | Window menu, "Next Window" (Cmd+`) | `desk.rs` `desk_cycle_window` + `cmdtab_release` (commits the second most recent window at once); `hwkeys.rs` `Shortcut::CycleWindow` (not in a terminal) |

Fixed-size app windows (`window_size = "fixed:…"`) are not tiled; `Desk::tile` returns early for them.

## How to verify on Midget

1. Open Settings and Terminal (`bifrost pc midget open-app ...`), then `bifrost pc midget screen /tmp/a.png`.
2. Window menu: the tile rows appear after Show Desktop. Choose Left Half; the front window moves to the left
   half and the screenshot shows it there. Choose Return to Previous Size; it goes back to its size.
3. Keys: press Ctrl+Cmd+Right, then Ctrl+Cmd+Up (Fill), then Ctrl+Cmd+Down (Center).
4. Snap: drag a title bar (`bifrost pc midget tap-window APP X Y` for the press, `swipe` for the drag) to the
   left edge and release; the window tiles to the left half.
5. Cmd+`: with two windows open, the other window comes forward.

## Host tests

`springboard/src/shell/desk.rs` test `tiling_moves_a_window_to_a_half_and_return_restores_its_size` (tile left,
return to previous, fill). Existing `tiles_cover_the_work_area_and_snap_zones_pick_edges` covers the tile rectangles
and snap zones. No build was run in this wave; the lead builds once.

## Evidence

- REA official page capture of NSWindow: `ev_fc1327896bb293f1e0143eb241e70f1474e4df48b18a75273880a60bffbca0ff`
  (AppKit window documentation, used for the tile and zoom behaviour).
- Official docs read with `tools/parity/docc.py`: `appkit/nswindow`, `appkit/nswindow/zoom(_:)` (toggles between the
  standard and user state), `appkit/nswindow/performzoom(_:)`.
- REA evidence already on the rows: `ev_f82ebd8719732e253617f0fa4a235954eb9b61fb56c2c20aeaeb04b009140b94`
  (appkit-user-interface), `ev_13fb77cf2a8baeb76d5ba523cb7154f39cc259d6e99eaf466bc087940ef72843` (appkit-app-structure).

## Remaining gaps

- Window tabbing (Merge All Windows, tab bar): no tab model exists for desktop windows; not wired.
- Full screen as a separate Space: not modelled. Full screen stays the zoom (Ctrl+Cmd+F).
- Minimize to Dock already works (`desk_minimize_window`); window restoration on login is in `restoration.rs` and was not changed.
- Snap has no drop preview overlay yet; the window moves and then tiles on release.
- Keyboard tiling is not available inside a terminal full screen (the terminal takes the keys).

## Matrix rows

| slug | feature (as in docs/parity-matrix/data) | BifrOSt path |
|---|---|---|
| appkit-user-interface | Windows, Panels, and Screens | springboard/src/shell/desk.rs (tile, snap, zoom, cycle) |
| appkit-user-interface | Menus, Cursors, and the Dock | springboard/src/shell/desktop_bar.rs (Window menu) |
| appkit-user-interface | Mouse, Keyboard, and Trackpad | springboard/src/shell/hwkeys.rs (Ctrl+Cmd arrows, Cmd+`) |
