# Parity wave 4: Screen Saver

Framework: Screen Saver (`docs/parity-matrix/data/screen-saver.json`). Official doc:
https://developer.apple.com/documentation/screensaver

## What was built

Two gap rows are closed to `partial`. The module model is pure Rust and std only, with host unit tests. Nothing is
wired into the shell, the window server or Settings yet, and nothing has run on the VM or Midget.

- `springboard/ui/src/screensaver.rs` (extends the wave 3 model):
  - `BackingStoreType` (`Retained`, `Nonretained`, `Buffered`, default `Buffered`) and
    `ScreenSaverView::backing_store_type` / `set_backing_store_type`, mirroring `backingStoreType()`.
  - `ScreenSaverView::perform_gamma_fade` / `set_perform_gamma_fade`, mirroring `performGammaFade()`. The reference
    page gives no default; the model starts with `false`.
  - `ScreenSaverView::has_configure_sheet` / `set_has_configure_sheet`, mirroring `hasConfigureSheet`.
  - `ConfigureSheet` with `SaverValue` (`Bool`, `Int`, `Choice`): the options a configuration sheet edits. `add_bool`,
    `add_int`, `add_choice` refuse empty or duplicate keys, empty ranges and defaults outside the range or list.
    `set` checks the kind, the range and the choice list, and changes nothing on a refused value. `values` gives the
    pairs for the caller to save to the module's preferences.
  - `preview_rect(screen, thumb)`: fits a screen's aspect into a preview thumbnail and centres it with the existing
    `ss_centered_rect_in_rect`.
- `springboard/render/src/gamma_fade.rs` (new): `GammaFade` and `FadeDirection`. `level` gives the brightness in 0..=1
  for a fade in or out after an elapsed time, linear and clamped; `ramp` gives evenly spaced levels across the fade.
  A fade with no valid length jumps to its end. The caller applies the level to the display gamma; no display code is
  here.

## Official API names mirrored

- `ScreenSaverView`: `backingStoreType()`, `performGammaFade()`, `hasConfigureSheet`, `configureSheet` (window, not
  modelled), `isPreview`.
- `ScreenSaverDefaults` is not changed in this wave.

## REA evidence

- `ev_fade3b2af28b29906215073eaf58378c47f9add4255b7579729be9411655a4b9`: Screen Saver landing page
  (https://developer.apple.com/documentation/screensaver). Confirms the Screen Saver View and Screen Saver Defaults
  classes and the utility functions.
- `ev_8756f392e9f4ddd0813f571aa3c6ba166a17853651cf1ffe4e7491f4f242115d`: `hasConfigureSheet`
  (https://developer.apple.com/documentation/screensaver/screensaverview/hasconfiguresheet). The module overrides it to
  return true when it ships a configuration sheet.
- `ev_f6b95d4c6901f9120d2160359f3535dc4bc6ad384d1ae95a1715c8a9448ddd2e`: `performGammaFade` page returned "page not
  found" in REA. The behaviour is taken from the DocC abstract: "Indicates whether to perform a gradual screen fade
  when the system starts and stops your screen saver's animation."
- DocC (`python3 -I tools/parity/docc.py screensaver/screensaverview`) for the method list and abstracts.

## Matrix rows closed

| Framework slug | Feature (as in data/screen-saver.json) | New status | BifrOSt path |
|---|---|---|---|
| screen-saver | Preview and configuration sheet | partial | springboard/ui/src/screensaver.rs |
| screen-saver | Window backing store and gamma fade for the saver window | partial | springboard/render/src/gamma_fade.rs; springboard/ui/src/screensaver.rs |

Not closed: `Preference storage for a screen saver module` (partial, not in this wave's gap scope; the per-module
domain is still missing), and the three `os` rows (idle, auto-lock, sleep inhibitors), which need an app-facing
SDK API and a shell host that this wave does not add.

## Limits

- The configuration sheet is a model. There is no window for it and no Settings page or preview thumbnail in the
  shell.
- The gamma fade is a ramp. Nothing applies it to the display, and the compositor has no hook for it.
- The backing store is a stated preference. The shell still draws every window into a buffer.
- Host tests only. `cargo` was not run in this round, per the wave rules.
