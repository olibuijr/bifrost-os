# Screen Saver wiring (wire 3)

Settings > Screen Saver now works on the desktop: a user picks a saver, sets how long the computer must be idle before
it starts, and can preview it. The saver covers the whole screen, and the first touch or key press returns the desktop
(or the lock screen, when the display has gone off). The models come from the screen-saver waves (`w3`, `w4`, `w5`).

Branch `wire3/screen-saver`, cut from `main` (0.5.38). Nothing was built or run (wire rules: the lead builds once).

## What is now wired

- **Saver choice and Start After** (Settings page): `Clock`, `Colours`, `Black`, and `1 minute` to `30 minutes`, or
  `Never` (the default, so an existing install behaves as before). Both are saved in `settings.ini` as
  `screensaver_module` and `screensaver_minutes` through `Options`.
- **Preview**: a row on the same page starts the chosen saver full screen at once, whatever the idle time. Any input ends it.
- **Idle activation** (`springboard/src/shell/screensaver.rs`): the shell feeds its last-input time to the existing
  `bifrost_core::idle_monitor::IdleMonitor`. When the idle timeout is reached, `IdleMonitor::poll` reports `BecameIdle`
  and the saver starts. Input that comes later ends it. The display sleep timer (Auto-Lock) still runs, and a saver stops
  as soon as the display goes off.
- **Frame clock and random helpers**: the saver's timing uses `bifrost_ui::screensaver::ScreenSaverView` (`start_animation`,
  `tick`, `stop_animation`, `animate_one_frame`, 30 frames a second, catch-up capped at 4). The Clock saver moves its clock
  with `ss_random_point_for_size_within_rect`, on a fixed hash of the minute cycle, so the same cycle always gives the same spot.
- **Unlock**: the saver does not change lock state. The dismissing touch or key is swallowed, and the screen beneath comes
  back: the desktop, or the lock screen through the existing lock path after Auto-Lock. A touch within 400 ms of a
  saver starting is ignored, so the touch that opened a Preview does not end it.

## How a user reaches it

Settings > Screen Saver. The row sits under Wallpaper on the root page (its value is `Never`, or the saver's name when
Start After is set). The search index also finds it as "Screen Saver".

## Shell hooks (minimal, in shared files)

- `springboard/src/shell/mod.rs`: `pub mod screensaver;`, a `saver` field, `saver_tick` before the display sleep check in
  `tick_inner`, `saver_wake` at the top of `event` (not for `Event::Power`), `saver_draw` at the top of `draw`, the
  `configure` call in `apply_options`, and `saver_preview` after a Settings touch that tapped Preview.
- `springboard/src/shell/options.rs`: two keys (`screensaver_module`, `screensaver_minutes`) in the struct, defaults,
  `KEYS`, `load` and `save`.
- `springboard/src/shell/pages.rs`: the `ScreenSaver` page, the root row, three `Pick` variants, and tests.
- `springboard/src/shell/settings.rs`: the `Pick` arms, the preview flag (`take_screensaver_preview`), the page name
  routing and the search entry, and a navigation test.

No wire protocol (`springboard/proto`) and no SDK change.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget state`.
2. Open Settings from the Dock, then Screen Saver. Check that the page lists Clock, Colours and Black, with Clock checked,
   and that Start After has `Never` checked.
3. Tap Colours, then Preview. `bifrost pc midget screen /tmp/saver-preview.png` should show the colour drift, full screen.
   Press a key with `bifrost pc midget key enter` (or touch the screen). The Settings page should come back.
4. Tap Clock, then `1 minute` under Start After. Leave the computer untouched for one minute and take
   `bifrost pc midget screen /tmp/saver-clock.png`. The clock should be on black, in the status-bar time. Move the mouse or
   tap, and the desktop comes back at once.
5. With a passcode set, let the Auto-Lock time pass after the saver starts. The display goes off, and the lock screen
   shows after the next touch, as before.
6. Dark and Light: the Clock saver is black in both, the Colours saver is unchanged.

## Evidence (official reference)

Reused from the screen-saver waves; this round did not fetch new REA pages.

- `ev_a115b6cf856edf0d27b365ece986ed09269d9736665f16a246f48238ff73efd0`: Screen Saver landing page
  (https://developer.apple.com/documentation/screensaver), from w3.
- `ev_fade3b2af28b29906215073eaf58378c47f9add4255b7579729be9411655a4b9`: Screen Saver landing page, from w4.
- `ev_8756f392e9f4ddd0813f571aa3c6ba166a17853651cf1ffe4e7491f4f242115d`: `hasConfigureSheet`, from w4.
- `ev_5f3cf45c762650c0f8c905ee847af78b40bc8fbf3b98ff5706ca513f3528b185`: Screen Saver, from w5.
- `ev_38b6561204c39e27a5c92ba51dbd8bd9ef78328cf5e7950ecb75b145b54f9b52`: ProcessInfo.ActivityOptions, from w5.
- `ev_cc2a3e312b2679aab08837530b31bc4264f252a6ac1417506af9f1d0610ddb94`: the matrix's evidence for the Screen Saver row.

The official pages give no default for `performGammaFade()` or the range rules for the random helpers. The choices are
recorded in the w3 and w4 notes. Start After's `Never` default is a BifrOSt choice, so existing installs keep the old behaviour.

## Remaining gaps

- No thumbnail of the saver in Settings. Preview is full screen only (`preview_rect` is not drawn).
- The configure sheet (`ConfigureSheet`, `hasConfigureSheet`) is not wired. The Clock saver has no options.
- `ScreenSaverDefaults` (per-module preferences) is not used; the shell keeps its two keys in `settings.ini`.
- No plug-in host: no `.saver` bundle loading, no saver window type, no SDK `ScreenSaverView` for apps. The saver
  is the shell's own.
- No gamma fade and no backing store choice (`performGammaFade`, `backingStoreType`): the models exist and nothing calls them.
- Sleep inhibitors (`springboard/core/src/sleep_assertions.rs`) are not read: an app cannot stop the saver yet.
- Idle state is not exposed to apps through the SDK.
- Not built, not tested on Midget. The unit tests are in `springboard/src/shell/screensaver.rs` (idle timing, the
  grace period, preview, display-off stop, index clamp, hash range) plus the page test in `pages.rs` and the Settings
  test `screen_saver_picks_persist_and_preview_is_queued_once` in `settings.rs`.

## Matrix rows touched (`docs/parity-matrix/data/screen-saver.json`)

For the lead to update when building the matrix. Statuses are left to the lead.

| slug | feature (as in the matrix JSON) | BifrOSt path |
|---|---|---|
| screen-saver | ScreenSaverView (plug-in view and animation lifecycle) | springboard/src/shell/screensaver.rs (drives bifrost_ui::screensaver::ScreenSaverView) |
| screen-saver | Preview and configuration sheet | springboard/src/shell/pages.rs (Screen Saver page, Preview row); configuration sheet not wired |
| screen-saver | Random value and rectangle helpers | springboard/src/shell/screensaver.rs (ss_random_point_for_size_within_rect, Clock saver) |
| screen-saver | Idle detection that starts the saver | springboard/src/shell/screensaver.rs (IdleMonitor from springboard/core/src/idle_monitor.rs) |
| screen-saver | Window backing store and gamma fade for the saver window | not wired |
| screen-saver | Preference storage for a screen saver module | not wired (shell keys in springboard/src/shell/options.rs) |
| screen-saver | Sleep inhibitors held by apps | not wired |
