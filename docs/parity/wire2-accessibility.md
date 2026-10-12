# Wire 2: Accessibility > Dim Flashing Lights

Branch `wire2/accessibility`, from `main` (release 0.5.37). Wires the flashing-lights model
(`springboard/ui/src/flashing_lights.rs`, wave 6 b13) into the desktop shell and the SDK. No builds were run in this
round (the lead builds once). No protocol change: the wiring uses the named service channel.

## What is now wired

- **Setting.** `AccessibilitySettings::dim_flashing_lights` (`springboard/src/shell/accessibility/mod.rs`), stored in
  `settings.ini` as `a11y_dim_flashing_lights` (`1` or `0`, default off). Loaded and saved with the other
  accessibility keys; the save-and-load round trip test covers it.
- **Settings page.** Settings > Accessibility > VISION gets a `Dim Flashing Lights` switch after `Show Borders`
  (`springboard/src/shell/pages.rs`, `Toggle::DimFlashingLights`; the toggle handler is in `settings.rs` next to the
  other accessibility toggles).
- **Service.** `accessibility_display` (`springboard/src/shell/services/accessibility_display.rs`, one line in
  `services::all()`):
  - `get()` returns `true` or `false`.
  - `changed` is pushed to every running app with the new flag as its body, once per change. The first tick sends
    nothing.
- **SDK client.** `bifrost_app::accessibility_display` (`springboard/sdk/src/accessibility_display.rs`):
  - `get(ctx)` asks for the flag; `from_reply` decodes the reply; `changed(service, event, body)` decodes the event.
  - `flashing_lights(dim_on)` returns a `FlashingLights` model with dimming set to the setting. The model is re-exported
    (`Sample`, `SWING`, `MAX_FLASHES_PER_SECOND`, `DIM_FACTOR`).

## How a user reaches it

Settings > Accessibility > VISION > Dim Flashing Lights. The switch is saved at once, like the other accessibility
switches.

## How an app uses it

```rust
let id = accessibility_display::get(ctx);                 // at start; reply via App::service_reply
// from_reply(result) -> Ok(true) means the user wants dimming
// on every change: accessibility_display::changed(service, event, body)
let mut model = accessibility_display::flashing_lights(dim);
let flagged = model.analyse(&samples);                    // the app supplies the timed luminance samples
let out = model.output(luminance);                        // DIM_FACTOR only when the setting is on and the picture is flagged
```

## How to verify on Midget (test PC)

1. `bifrost pc midget state`, then open Settings and go to Accessibility.
2. Tap the `Dim Flashing Lights` switch with `bifrost pc midget tap-window Settings X Y` (window-local pixels). The
   switch turns on. Check `settings.ini` on Midget for `a11y_dim_flashing_lights=1`.
3. Restart the shell (or reopen Settings): the switch keeps its state.
4. Run an SDK test app that calls `accessibility_display::get` (for example through `bifrost pc control call` if the app
   has a control method). It should report `true`, then `false` after the switch is turned off. An app that keeps the
   service event handler should also see one `changed` event per switch.

## REA evidence

- Official "Flashing lights" page (the setting, the notification, the processor and the check function):
  `ev_6d3ca2a5aa7b9b2e7b9c292467f92f868784bede6c4e5350d1e9e74a49c1b774`
  (`https://developer.apple.com/documentation/mediaaccessibility/flashing-lights`). It says apps should respond to
  changes in the setting and update the UI, and that the framework's processor works on a framebuffer.
- Official Media Accessibility collection (`tools/parity/docc.py mediaaccessibility`): lists Flashing lights,
  Captions and Music Haptics as its features. The matrix row's REA evidence is
  `ev_638847f577c821df90ff1e7bb3d494e65444e64de6b02caaf879bc750ba14bc6`.

## Gaps (not wired in this round)

- **The shell's own video is not dimmed.** The shell's video path is the media player (`springboard/src/shell/video.rs`,
  used by the Deildu player). Dimming a decoded frame needs a pass in the GPU presenter, and that is not in this round.
  The setting is published to apps; the shell does not change its own video output yet.
- **No frame analysis in the shell.** The shell does not read app pictures. Apps that show flashing content run the
  model themselves with luminance samples they take from their own frames.
- **Flash count is simplified.** The model counts reversals, not the full pair-of-transitions rule of the photosensitivity
  guidelines (documented in `flashing_lights.rs`).
- **No setting in the Settings search index.** The new row is found by the page, not by `settings_search`.
- **Midget not tested.** Host unit tests were written by hand; the lead runs them. The Midget steps above are not yet run.

## Matrix rows

| Matrix slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| media-accessibility | Flashing lights detection and dimming | springboard/ui/src/flashing_lights.rs (model); springboard/src/shell/accessibility/mod.rs (setting); springboard/src/shell/services/accessibility_display.rs (service); springboard/sdk/src/accessibility_display.rs (SDK) |
