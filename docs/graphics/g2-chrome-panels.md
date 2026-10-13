# G2 chrome-panels: Notification Center, Control Center and search as a client

Owner: wave G2 agent `chrome-panels`. Branch `g2/chrome-panels`.

## What is implemented

- `springboard/chrome/src/panels.rs`, the pure logic of the panels client, host-tested (12 tests):
  - `parse_panel`: `ShowPanel` names (`notifications`, `control`, `search`); `""` and unknown names close.
  - `panel_rect`: where each panel sits. Notification Center and Control Center hang from the top right under the
    menu bar (8 px margin); search is centred 52 px tall (the capsule field). Widths clamp to narrow outputs.
  - `group_notifications`: notifications grouped by app id, newest first inside a group, groups ordered by their
    newest id (ties by app id).
  - `control_layout`: Control Center tiles. Toggles (`value < 0`) pair two to a row, sliders (`value >= 0`) take a
    full row. Returns each tile's rect and the content height.
  - `progress`, `slide_offset`, `fade_alpha`: the open and close animation curves (240 ms, ease-out cubic slide,
    linear fade). The window server runs the animation; these functions give the same numbers for the client's
    own layout and tests.
  - `toggle`, `set_value`: apply a Control Center edit to the model and return the `FromChrome::ControlChanged`
    message for the shell (sliders clamp to 0..1).
  - `run`: returns an error that says the surfaces are not wired yet. It replaces the old `todo!` so the shell can
    fall back cleanly.
- `springboard/src/shell/chrome_host/panels.rs`: `model(store, now)` builds the `PanelsModel` for Notification
  Center from the shell's notification store (app id = the app's bundle id, age text from `age_text`).

## Mapping to the contract

- `model::PanelsModel`, `ControlModule`, `Notification`, `FromChrome::ControlChanged` are used as declared. No
  change to `model.rs`.
- `ShowPanel` (`ToChrome`) is handled through `parse_panel`.
- Glass material (`bifrost_layers::Material::Glass` on the panel layer) and the layer-shell surfaces are not
  written: they need `surface::Display` and `link`, which are still `todo!("chrome-core")` on `main`.

## Not done (todo_left)

1. Layer-shell surfaces for the three panels (`Layer::Overlay`, keyboard focus for search), drawn with `bifrost-2d`
   and committed as layer trees, with the glass material and the slide and fade animations run by the server.
   Blocked on `chrome-core` (`surface.rs`, `link.rs`).
2. `chrome_host/panels.rs` does not yet build Control Center modules. The shell's `control_center.rs` module list
   must be mapped to `ControlModule` (toggles and sliders).
3. `NotificationAction`: the store keeps no per-notification action list, so the producer sends no actions and the
   handler is not written. The store id is `u64`; `model` truncates it to the `u32` the protocol uses.
4. The shell-side code is compile-checked only. No unit test covers `chrome_host::panels::model`, because the
   springboard test build was not run in this round (the build rules allow only `check`).

## How to verify

Host (pure logic), from `springboard/`:

    CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-chrome --tests

The owner runs the tests with `cargo test -p bifrost-chrome panels`.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4). The session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says
`bifrost-wm`. Once the surface API lands: set `/etc/bifrost/compositor` to `bifrost-wm`, set
`BIFROST_WM_RENDERER=vulkan BIFROST_CHROME=1` and start `bifrost-panels`; send `ShowPanel("notifications")` and
check the panel slides in and the glass blurs the desktop behind it. Until then `bifrost-panels` exits with the
"not wired yet" message.
