# Wire: `screens` system service (AppKit Windows, Panels, and Screens)

Branch `wire/svc-screens`. Matrix row: `appkit-user-interface`, feature "Windows, Panels, and Screens" (was Partial,
"NSScreen-style multi-screen API for apps is thin"). Model reused: `springboard/ui/src/screen.rs` (`Screen`, `Screens`,
`Align`), no new model.

## What is now wired

- **Shell service** `screens`, `springboard/src/shell/services/screens.rs` (`Displays`, implements `SystemService`),
  registered by one line in `services/mod.rs`. No protocol change: apps reach it over `ToShell::ServiceCall`.
  - `list`: one object per display: `name`, `frame` {x,y,w,h}, `visible_frame` {x,y,w,h}, `scale`, `main`.
    Coordinates are global points. The frame is the framebuffer divided by the interface scale (`layout.s`). The
    visible frame leaves out the menu bar and Dock in desktop mode (`desk_metrics().menubar` and `dock_bottom`), or the
    status bar on home. `scale` is the backing scale the shell renders at.
  - `changed` event: pushed to every running app (`app: None`) from `tick` when the list differs from the last one
    seen. The first tick is a baseline and sends nothing. The body is the new list.
- **SDK client** `springboard/sdk/src/screens.rs`, exported as `bifrost_app::screens`:
  - `list(ctx) -> u32` sends the call; the reply comes to `App::service_reply`, or to `service::take_reply`.
  - `from_reply(&Result<Json,String>) -> Result<Screens,String>` decodes a reply (`bifrost_ui::screen::Screens`, so
    `main`, `containing`, `containing_rect`, `to_backing` and `backing_aligned` are available to the app).
  - `changed(service, event, body) -> Option<Result<Screens,String>>` decodes the pushed event.
  - Constants `SERVICE`, `METHOD_LIST`, `EVENT_CHANGED`.

## How a user reaches it

Apps only. There is no UI in the shell yet; an app that wants display geometry (for example a window that re-lays out
when the desktop resizes, or one that places a panel on the main screen) calls `screens::list` at start and handles
`changed` through `App::service_event`. Nothing is shown to the user by this change alone.

## Verify on the test PC (Midget)

Host tests (run by the lead, not built here): `cargo test -p springboard screens` covers the geometry snapshot, the
list encoding, and the change detection (`next_event`); `cargo test -p bifrost-app screens` covers decoding, the
main flag, clipping, malformed input and the event filter.

On Midget: `bifrost pc midget state` should show the desktop; then, from an app that calls `screens::list`, the
returned frame must equal the screen size in points (for example 1440x900 at scale 2 for a 2880x1800 panel) and the
visible frame's `y` must equal the menu bar height in points. To check `changed`, resize the desktop (Settings >
Display, or a scale change) and confirm the app receives one event with the new frame. Per the owner rule, Midget
only, no VM.

## REA evidence

- `ev_468c372e2f481e3f4e5a671a863f8298ef92ac159a3a058a230d4ff07aa1aea4`: rendered official page for NSScreen
  (`https://developer.apple.com/documentation/appkit/nsscreen`) via `tools/parity/rea-page.sh`, read this round.
- `ev_ec5ff1350de1641a467b991d69b4d860787a5eb12673ccc3f9f7bde6efd81272`: the earlier NSScreen capture recorded in
  `docs/parity/appkit-user-interface.md`.
- Official symbol list from `python3 -I tools/parity/docc.py appkit/nsscreen` (screens, main, deepest, frame,
  visibleFrame, backingScaleFactor, maximumFramesPerSecond, screensHaveSeparateSpaces, and others).

## Remaining gaps

- **Refresh rate** (`maximumFramesPerSecond`, refresh interval): not reported. The Wayland output mode is kept inside
  `bare/wayland.rs` and does not reach the shell. Needs the backend to pass the mode to `Shell`.
- Several displays: the shell drives one panel, so `list` has one entry and `main` is always it. `Screens::update`
  and `set_main` in the model are ready for more.
- `deepest`, `supportedWindowDepths`, colour space and gamut (`depth`, `colorSpace`, `canRepresent`): not reported.
- Localized display name: the name is the fixed string `Display`.
- Safe-area insets, auxiliary top areas, EDR values, `displayLink`, `displayUpdateGranularity` and
  `screensHaveSeparateSpaces`: not modelled.
- `backingAlignedRect` and conversions work on the reported `scale`; the shell's own rendering is not changed.
- No shell UI yet (Displays settings, a screen picker for windows).
- Not built or run here (NO-build rule). A build and host test pass, then a Midget run, are still needed.
