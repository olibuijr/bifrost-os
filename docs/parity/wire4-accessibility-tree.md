# Wire 4: Accessibility tree (hierarchy, announcements, custom actions)

Branch `wire4/accessibility-tree`, from `main` (release 0.5.40). Wires three accessibility rows into the shell's element map
(`bifrost pc midget ax`, the `ax` control command) and the on-screen announcement banner. No protocol change: apps reach the
shell through the existing `ControlsTree`, `Announce` and `AxText`/`AxMeta` messages. No builds were run in this round (the
lead builds once). Host unit tests were written by hand; they have not been run.

## What is now wired

1. **Element hierarchy, parent, focus and checked state in `ax list`.** `springboard/src/shell/accessibility/tree.rs`.
   - Each listing line now has eight tab-separated columns: `address, role, name, value, enabled, parent, checked, focused`.
     The first five are unchanged, so agents that read them keep working. `parent` is the address above the element when that
     address is itself an element (empty when the element sits directly in its window). `checked` is `checked`, `unchecked`,
     or empty when the element is not checkable. `focused` is `focused` for the element that holds keyboard focus.
   - An app's `ControlsTree` (`NodeExt`) now nests its addresses (`app/<id>/list/files/report`) and reports focus and the
     checked state. With no tree, app addresses are the same as before (`Map::add_nodes`).
   - Settings toggles report `checked` from their value (`on` or `off`).
   - The shell's `ax_map` (`springboard/src/shell/axmap.rs`) now builds the state beside the map (`ax_state`).
2. **Announcements on screen.** `springboard/src/shell/accessibility/banner.rs`.
   - An app's `announce` call (the SDK `Ctx::announce`, already present) is now also shown as a banner at the top centre of the
     screen for 4 s (`SHOW_MS`). The screen reader still speaks it through AT-SPI as before.
   - Assertive announcements (`Priority::High`) show before waiting polite ones. The same text twice in a row shows once. Blank
     text is refused. The queue is `bifrost_ui::ax::Announcements`.
   - The banner is a dark pill whatever the appearance, so its white text always reads.
3. **Custom actions on windows: listed and invokable.** `springboard/src/shell/accessibility/actions.rs`.
   - Each open window is an element `os/windows/<app>` (role `window`, value `open` or `minimized`, handle `win:<index>`).
   - Its custom actions are `Zoom`, `Minimize` and `Close`, the same actions as its title bar.
   - `ax actions <element>` lists them; `ax action <element> :: <name>` runs one. `ax press` on a window refuses, and says to
     use `ax action`.

## How a user reaches it

- **Hierarchy and state:** `bifrost pc midget ax list os/settings/general` (the new columns are at the end of each line).
  Toggles show `checked` or `unchecked`.
- **Announcements:** an app calls `announce` (SDK `Ctx::announce(text, priority)`); the banner appears at the top of the
  screen. Settings is not an announcing app, so use a test app that calls it.
- **Window actions:** `bifrost pc midget ax list os/windows` lists open windows; `bifrost pc midget ax actions os/windows/settings`
  lists `Zoom`, `Minimize` and `Close`; `bifrost pc midget ax action os/windows/settings :: Minimize` runs one.

## How to verify on Midget (test PC)

1. `bifrost pc midget ax press os/apps/settings` (opens Settings).
2. `bifrost pc midget ax list os/windows`: one line `os/windows/settings  window  Settings  open  enabled  (empty parent)
   (empty checked)  (empty focused)`.
3. `bifrost pc midget ax actions os/windows/settings`: prints `Zoom`, `Minimize`, `Close`, one per line.
4. `bifrost pc midget ax press os/windows/settings`: answers an error that says the element is a window.
5. `bifrost pc midget ax action os/windows/settings :: Minimize`: answers `ok os/windows/settings`; the window goes to the Dock.
   `ax list os/windows` now shows value `minimized`.
6. `bifrost pc midget ax list os/settings/general` on the Software Update page: toggle rows show `checked` or `unchecked`
   in the seventh column.
7. Announcements: run an SDK test app that calls `announce("Saved", Priority::Medium)`. A dark banner "Saved" should appear
   at the top of the screen for about 4 s, then go. A second announcement with `Priority::High` should show before any waiting
   polite one.

## REA evidence

Official pages rendered and recorded with `tools/parity/rea-page.sh`:

- Accessibility collection (`https://developer.apple.com/documentation/accessibility`):
  `ev_2cd6d5dc5ba116291b186263ebf1503997a1ba87641ccbcf7cf3c9e0f5ec8ceb`
- Accessibility Inspector (`https://developer.apple.com/documentation/accessibility/accessibility-inspector`):
  `ev_7b0cc149a8095e216a8f8e3cab7670f084472241ade8b122ebda267bd0b78e35`

The `tools/parity/docc.py accessibility/accessibilitynotification` listing (not a REA capture) names the
`AccessibilityNotification` cases: Announcement, LayoutChanged, ScreenChanged and PageScrolled. The announcement behaviour
(low, medium and high priority) comes from the SDK model `bifrost_ui::ax::Announcements`, which predates this round.

Two candidate pages (`accessibility-announcements` and `nsaccessibility/accessibility-custom-actions`) returned "page not
found" and were not used.

## Gaps (not wired in this round)

- **App custom actions.** Only the shell's window actions exist. An app cannot yet declare its own custom actions: the
  `NodeExt` message has no field for them, and the wire protocol must not change. The next step is an accessibility-actions
  service on the service channel, where an app declares names and receives an invocation event.
- **Value setting and increment.** `AxAction::SetValue` and `Focus` are not routed by `ax` yet. Only `press` and the window
  actions run.
- **Focus is reported, not moved.** The shell reports which element an app marks focused. `ax` cannot yet move focus.
- **Banner timing.** The banner clears on the first shell frame after its time is up. An idle shell with no frames keeps the
  text until the next frame.
- **Screen reader not verified.** AT-SPI announcements are not checked on a live Orca. The banner is not spoken twice: the
  AT-SPI bridge still takes the original queue.
- **No app announces yet** through `ToShell::Announce` in the shipped apps; the banner is only exercised by an SDK test app.
- **No tests run.** The unit tests in `tree.rs`, `banner.rs`, `actions.rs`, `control.rs` and the shell test
  `an_open_window_lists_and_runs_its_title_bar_actions` (desktop-wayland feature) are written but not compiled.

## Matrix rows

The matrix data file is `docs/parity-matrix/data/accessibility.json` (the matrix slug is the data file name). The lead updates
the rows; this round does not edit `docs/parity-matrix/`.

| Matrix slug | Feature (as written in `docs/parity-matrix/data/accessibility.json`) | BifrOSt path |
|---|---|---|
| accessibility | Element hierarchy (parent, focus, checked state) | springboard/src/shell/accessibility/tree.rs (listing, add_app_tree); springboard/src/shell/axmap.rs (ax_state); springboard/src/bare/control.rs (ax list) |
| accessibility | Accessibility announcements | springboard/src/shell/accessibility/banner.rs (on-screen banner); springboard/src/shell/apphost.rs (announce_banner queue); springboard/src/shell/mod.rs (tick, draw) |
| accessibility | Custom actions and value setting | springboard/src/shell/accessibility/actions.rs (window actions); springboard/src/shell/axmap.rs (ax_actions, ax_action); springboard/src/bare/control.rs (ax actions, ax action) |
