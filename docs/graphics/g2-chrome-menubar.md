# G2 `chrome-menubar`: the menu bar as a separate client

Part of wave G2 in `docs/graphics-waves.md` (row 15). Contract: `docs/graphics-architecture.md` ("Dock, menu bar,
SystemUIServer"). Files: `springboard/chrome/src/menubar.rs` (the client) and
`springboard/src/shell/chrome_host/menubar.rs` (the shell's model and handler), plus one accessor pair on `Row` in
`springboard/src/shell/desktop_bar.rs`.

## Status

Implemented and host-checked: the layout, the menu navigation, the model producer and the action handler, with
unit tests. Not yet running end to end: `Display::connect/create/draw/dispatch` and `Link` are still the skeleton's
`todo!("chrome-core")`, and `chrome_host/server.rs` (`chrome-core`) does not yet call `menubar::model` or
`menubar::menu_action`. Until both land, `bifrost-menubar` panics on its first call into `Display`.

## What is implemented

### Client (`chrome/src/menubar.rs`)

- **Bar layout** (`layout_bar`, points). `menus[0]` is the system mark (34 pt at 6 pt from the left). Each other menu is
  its title plus 12 pt on each side. Status items are placed from the right edge (16 pt inset) leftwards, and an item
  that would come within 8 pt of the menus is dropped, leftmost first, as the shell's bar does. A status item without
  text takes the 30 pt icon width.
- **Popups** (`layout_popup`). Rows are 24 pt, separators 11 pt, padding 5 pt, under the bar with a 6 pt gap. The
  width is the widest row (18 pt check column, label, 24 pt gap and shortcut, 16 pt right inset), at least 180 pt, and
  the popup moves left to stay on the screen.
- **Navigation** (`Nav`, pure). Open a slot, step up and down over selectable rows (skipping separators and disabled
  rows, wrapping), switch to the neighbouring menu with Left and Right, type-select by first letter, hover to
  highlight, and activate to get the `(menu id, item id)` pair for `FromChrome::MenuAction`.
- **Input.** A bar click opens or closes the slot under the pointer. Hover over the bar moves an open menu to the slot
  under the pointer. A click on a popup row activates it when the row is selectable. Keyboard on the popup: Escape
  closes, Up and Down step, Left and Right switch menus, Return and keypad Enter activate, a printable key type-selects.
- **Surfaces.** The bar is a layer-shell `Top` surface anchored top, left and right, 28 pt high, with an exclusive zone
  of 28 pt. Each open menu is a layer-shell `Overlay` surface anchored top-left with its position as the margin. It
  takes keyboard focus, is moved with `update` when the open menu changes and destroyed on close.
- **Painting.** `Canvas` into a `DisplayList` (scale applied as a transform), colours from `render/src/ui.rs`
  (`BAR_TINT`, `BAR_TEXT`, `BAR_OPEN_CAPSULE`, `MENU_SOLID`, `MENU_HIGHLIGHT`, `MENU_HIGHLIGHT_TEXT`, `MENU_SHORTCUT`,
  `LABEL`, `LABEL_3`, `SEPARATOR`), text sizes `MAC_HEADLINE_PT` (titles) and `MAC_CALLOUT_PT` (status and rows). The
  face is `springboard/assets/SFUI.ttf`, measured with `bifrost_render::font::Font::measure`.
- **Loop.** `run()` connects the shell link and the display, creates the bar, then blocks on the display with the
  link's fd, applies the latest `ToChrome::MenuBar`, handles events and paints what changed. A closed bar or a lost
  link ends the client with an error.

### Shell side (`src/shell/chrome_host/menubar.rs`)

- `model(shell)` builds the `MenuBarModel` from `menu_buttons()` and `menu_rows()`. The menu id is the button's index
  and the item id is the row's index, so the handler needs no table. Menus go to `menus` (System first, with an empty
  title), everything else to `status`. Rows map to `MenuItem` through `RowView`: a check mark at the start of the text
  becomes `checked`, a row without an action is disabled, a separator has an empty label.
- `menu_action(shell, menu, item, t)` resolves the ids against the shell's current state and returns the row's
  `run_menu_action` effects. Unknown ids do nothing.

## Contract mapping

| Contract item | Where |
|---|---|
| `menubar::run` | `chrome/src/menubar.rs` `run()` |
| `MenuBarModel`, `FromChrome::MenuAction` | `chrome/src/model.rs` (unchanged) |
| Layer-shell bar with exclusive zone | `bar_spec()` |
| Overlay popups | `popup_spec()` and `sync_popup()` |
| `Canvas` + `DisplayList` drawing, GPU through `Display::draw` | `paint_bar`, `paint_popup` |
| Shell model producer and action handler | `chrome_host/menubar.rs` `model`, `menu_action` |

No contract change. Two assumptions the `chrome-core` side must match, and it is worth checking them when `surface.rs`
lands:

- `SurfaceSpec::margin` is `(top, right, bottom, left)` in logical pixels, measured from the top-left of the output.
- `Display::update` on an existing surface re-configures it and sends a new `Configured`. The client waits for that
  before drawing again.

## What is left (todo_left)

1. `chrome-core`: `Display` (connect, create, update, destroy, draw, dispatch), `Link` and `HostLink`, and the codec.
   The client does nothing until these exist.
2. `chrome_host/server.rs` must call `menubar::model(shell)` when a client says `Hello` or the menus change, and
   `menubar::menu_action(shell, menu, item, t)` on `FromChrome::MenuAction`, applying the returned effects.
3. Outside clicks do not close a menu yet. The contract has no pointer grab for overlay surfaces. Options: an
   exclusive-focus keyboard grab, a transparent full-screen backdrop surface, or the window server closing overlays
   on an outside press (`bifrost-wm`, `wm-input`). Until then a menu stays open until a bar click, Escape, or a choice.
4. Submenus are shown as plain rows; the chevron and the nested popup are not drawn or opened.
5. The system mark is a rounded square placeholder. The BifrOSt mark and the status symbols need the shell's symbol
   set (`springboard/src/symbols`), which the chrome crate does not depend on yet.
6. Status items draw only their text. The Wi-Fi, battery and input-source glyphs need the same symbol set.
7. Shortcuts are shown as the raw combo ("cmd+q"); the shell's spaced glyph form ("⌘ Q") is not reproduced.
8. Menu text uses the regular weight for titles (layout measures regular; the title is drawn at regular too). The
   semibold title weight needs `measure_var` with the `wght` axis in `layout_bar`.
9. The popup has no shadow: the shadow needs a margin around the surface, which the popup surface does not have.
   Add the margin with the popup's size and offset when the window server's shadows are in.
10. Menu radius is 6 pt (`MENU_RADIUS_PT`); check it against the design after the shell's menu values are confirmed.
11. Dead-code warnings in `chrome_host/menubar.rs` remain until item 2.

## Tests

- `chrome/src/menubar.rs` `tests` (14 tests): bar placement (menus, status from the right edge, dropping status items,
  icon width, hit testing), popup geometry (rows, separators, width, screen clamp), row navigation (skipping, wrapping,
  type-select), `Nav` (open, step, switch with wrap, disabled rows refused, activation ids), id lookups. Expected values
  are worked out by hand in the comments, with a seven-pixel-per-character measurer.
- `springboard/src/shell/chrome_host/menubar.rs` `tests` (7 tests): separators, check marks, disabled labels,
  shortcut carry-over, empty menus.

Commands run (from `springboard/`):

- `CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-chrome --tests`: passes. No warnings from the menu bar files.
- `CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p springboard --features desktop-wayland`: passes. The menu bar's shell-side functions warn as unused until item 2.
- `python3 -I tools/check-names.py` (from the repo root): no hits.

The tests were not run (`cargo test` is not part of this round's rules). The lead runs them.

## How to verify on Midget (once items 1 and 2 are in)

Midget is an Intel HD 530 with Mesa ANV and Vulkan 1.4. Its session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says
`bifrost-wm`.

1. Build the client and the shell with the fast profile: `cargo build --offline --profile fast --bin shell
   --features desktop-wayland` and `cargo build --offline --profile fast -p bifrost-chrome --bin bifrost-menubar`
   from `springboard/`.
2. Deploy with `tools/midget-push-shell.sh` (it pushes the shell) and copy `bifrost-menubar` to `/usr/bin` on Midget.
3. On Midget run the shell with `BIFROST_CHROME=1` and `BIFROST_WM_RENDERER=vulkan` under `bifrost-wm`, or under
   labwc with `BIFROST_CHROME=1` for the layer-shell path alone. Start `bifrost-menubar` from the session.
4. Check that the bar is at the top with its menus, status items at the right, and the exclusive zone pushes windows
   below it (open a window and see its top edge).
5. Click a menu title. The popup appears under it; hover moves to the next menu; Escape closes it; Up and Down move
   the highlight over the enabled rows only; Return runs the row (check the shell's effect, such as Settings opening).
6. Screenshot with `bifrost pc midget screen /tmp/menubar.png` and compare the bar and a popup with the shell's own
   bar (the old path with `BIFROST_CHROME` unset) in light and dark (`bifrost pc midget appearance light|dark`).
7. Record the bar's frame time from `bifrost-wm-frames.jsonl` when `wm-session` and `bench-graphics` land.
