# G2 `chrome-dock`: the Dock as a separate client

Owner: wave G2, row 16 of `docs/graphics-waves.md`. Files: `springboard/chrome/src/dock.rs` (the client),
`springboard/src/shell/chrome_host/dock.rs` (the shell's producer and handlers), and one additive field in
`springboard/chrome/src/model.rs`.

## What is implemented

Client (`bifrost-dock`, `chrome/src/dock.rs`):

- Geometry (`layout`): a bottom, left or right layer-shell surface anchored to its edge, with the capsule's exclusive
  zone (`exclusive`, 0 when autohide is on). Icons are centred along the edge. Sizes shrink to fit the output (minimum
  24 px). Margins and padding follow the shell's Dock (`MARGIN`, `PAD`, `GAP` = 10 pt).
- Magnification (`magnify_factor`, `icon_size`, `layout` with a pointer): the cosine falloff of the shell Dock, quantised
  to 2 px. The icons keep their edge and the row spreads from its centre. `approach` eases the amount toward the pointer
  at a frame-rate-independent rate.
- Launch bounce (`bounce_lift`, `lift_offset`): the shell's kinematics, one hop of `motion::duration(DockBounce)`,
  no bounce under Reduce Motion, gives up after 10 s. Started on a click of an app that is not running.
- Badges (`badge_rect`): a pill at the top right, text drawn with the SF UI font through `Font::measure`.
- Progress (`progress_fill`): a fraction fill, and an indeterminate sweep (`PROGRESS_PERIOD_S`) for the model's
  progress values.
- Drag (`Gesture`, `Outcome`): a press becomes a drag after 6 px; a release gives a click, a move to another slot
  (`Move { target, after }`), a remove (lift more than 80 px off the capsule) or nothing.
- Context menu (`context_menu`): Open / Show, Keep in Dock or Remove from Dock (checked when pinned), Quit when running.
  It opens as its own layer-shell surface above the capsule (`MenuSurface`, bottom Docks only).
- Minimize targets (`minimize_rect`, `target_changed`): every icon's rect is published as `MinimizeTarget` when it moves
  by more than a quarter of a pixel.
- `run()`: the event loop on `Display` and `Link`. Redraws when something changed or while animating. Icons are decoded
  when the model arrives (`DockState::set_model`).

Shell side (`src/shell/chrome_host/dock.rs`):

- `chrome_dock_model`: the producer. It takes the list `desk_dock` lays out, without the Trash and minimized windows,
  and gives each entry its key (`dock_app_key`), title, running state, pinned state (`dock_section == 0`), icon path
  (installed apps only), size, magnification, position and autohide.
- `chrome_dock_message`: `DockActivate` runs `Change::Open`. `DockCommand` decodes `show`, `quit`, `keep`, `remove` and
  `move:<key>:<0|1>` (`decode_command`, `split_move`). Pin changes run only when they change the state, since the
  shell's `Pin` toggles. A move calls `dock_reorder`. Messages for items the Dock no longer has are ignored.
  `MinimizeTarget` and `Hello` are not the Dock's to handle (the window server consumes the first).

## Mapping to the contract

- Surfaces and input: `SurfaceSpec` (`Layer::Top`, edge anchors, exclusive zone), `Display::create/update/destroy/draw`,
  pointer `Event`s and `Link::send/recv`. Icons, the capsule and text are a `DisplayList` drawn with `bifrost-2d`'s
  `Canvas` vocabulary (`fill_rrect`, `image`, `text`, `push_transform`).
- Layers: the architecture asks for icons as layers so magnification and bounces animate in the window server. This
  round does not do that (see todo). The Dock re-records its list at the frame rate while magnifying or bouncing.
- Minimize targets: `FromChrome::MinimizeTarget` (contract message).
- Model: `DockModel` / `DockItem` as the lead wrote them, plus `DockItem::pinned` (see contract changes).

## Contract changes

1. `model.rs`: `DockItem` gains `pub pinned: bool` at the end (additive). The codec (`chrome-core`) must carry it, or
   the context menu will always show "Keep in Dock".
2. Command strings (no new variants): `DockCommand { command }` uses `keep`, `remove`, `show`, `quit` and
   `move:<target app_id>:<0|1>`. The target is named by its key, not its index, so both ends agree even when the
   list changed. The shell decodes it in `chrome_host/dock.rs`.
3. Required from `chrome-core` (not done here, `surface.rs` is that owner's file):
   - `Display::bind_contents(layer, ContentsId, DisplayList or buffer)` so each icon can be a layer with its own
     backing store (needed for the window server to animate magnification and bounces).
   - The output's size or origin in `Event::Configured`, or in a new event. `MinimizeTarget` rects are currently
     surface-local, so the window server cannot aim at them without it.

## Todo left

- Icons are not layers. Magnification and the launch bounce are re-recorded by the client, so they are not yet at
  window-server rate. The layer transaction builder (`bifrost_layers::Transaction`, implicit animations over
  `Property::Bounds` and `Transform`) is not written, and it waits for the contents binding above.
- Minimize rects are surface-local (see contract change 3).
- The shell does not fill the `active`, `badge` or `progress` fields. The Dock service's tile state
  (`dock_tile_state`) is private to `desktop_dock.rs`, so the producer cannot read it without a visibility change in
  that file.
- Built-in apps have no icon file for the client (the shell draws them in-process from `icon_image`), so they show a
  placeholder tile. An icon export is needed.
- Section dividers (pinned, recents, minimized) are not in the model, so the client draws none.
- Minimized windows are not Dock items yet (the shell lists them; the client does not).
- The context menu is bottom-only. Side Docks ignore right clicks.
- Autohide is passed to the surface spec only; nothing slides the surface.
- Nothing has been run on Midget. The host tests were worked out by hand and checked against the code, but
  `cargo test` was not run (this round may only `cargo check`). The lead runs
  `cargo test -p bifrost-chrome dock` and `cargo test -p springboard chrome_host::dock`.
- The client's `run()` depends on `chrome-core`'s `Display`, `Link` and surface functions, which are still `todo!()`.

## How to verify on Midget

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4). The session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`. The chrome clients need `BIFROST_CHROME=1` and `BIFROST_WM_RENDERER=vulkan` (with `bifrost-wm`).

1. Build (fast profile): `cargo build --offline --profile fast --bin shell --features desktop-wayland` and
   `cargo build --offline --profile fast -p bifrost-chrome --bin bifrost-dock` in `springboard/`.
2. Host tests: `cargo test --offline -p bifrost-chrome dock` and `cargo test --offline -p springboard chrome_host::dock`.
3. Push and start: `tools/midget-push-shell.sh --no-build shot.png`. Then `ssh midget`, export `BIFROST_CHROME=1`,
   and run `bifrost-dock` against the running compositor.
4. Look at the bottom edge: the capsule, the icons, and the running dots. Hover over the icons: they should grow
   toward the pointer. Click an app that is not running: the icon should bounce once.
5. Right-click an icon: the menu appears above it. Choose Keep or Remove and check `desktop-dock.ini` for `pinned=`.
6. Drag an icon sideways: its neighbours move (the shell's reorder). Drag it up and away past 80 px: it is removed.
7. Minimize rects: while a window minimizes, the window server should aim the animation at the icon's rect. Until
   contract change 3 lands the rects are surface-local, so the aim is off by the output offset; that is expected for
   this round.
