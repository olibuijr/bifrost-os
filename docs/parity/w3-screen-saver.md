# Screen Saver parity (wave 3)

Three gap rows from the Screen Saver matrix (`docs/parity-matrix/data/screen-saver.json`) are now modelled in pure Rust.
Nothing here loads a screen saver bundle, draws a saver window, reads idle state or talks to the compositor. The
preview and configuration sheet, the backing store and gamma fade, and an app-facing idle signal stay gaps.

## What was built

- `springboard/ui/src/screensaver.rs` (`bifrost_ui::screensaver`): `ScreenSaverView`, a pure animation state machine.
  The caller's clock drives it through `start_animation(now)`, `tick(now)`, `stop_animation()` and
  `animate_one_frame()`. It keeps the `animationTimeInterval` setting (default 1/30 s, clamped to at least 1/120 s,
  and a non-finite or non-positive value keeps the default), and caps catch-up after a stall at 4 frames.
  Also the random utilities `ss_random_int_between`, `ss_random_float_between`,
  `ss_random_point_for_size_within_rect` and `ss_centered_rect_in_rect`. They take unit values in [0, 1) that the caller
  samples, so no generator is chosen here.
- `springboard/sdk/src/screen_saver_defaults.rs` (`bifrost_app::screen_saver_defaults`): `ScreenSaverDefaults`, a
  per-module view of the app's `UserDefaults`. A module's keys are stored as `ScreenSaver.<module>.<key>`, and a dot in a
  module name becomes `_`.

## Official API names mirrored

- `ScreenSaverView`: `init(frame:isPreview:)`, `isPreview`, `startAnimation()`, `stopAnimation()`, `isAnimating`,
  `animationTimeInterval`, `animateOneFrame()`.
- `ScreenSaverDefaults`: `init(forModuleWithName:)`.
- `SSRandomIntBetween`, `SSRandomFloatBetween`, `SSRandomPointForSizeWithinRect`, `SSCenteredRectInRect`.

Signatures come from the rendered reference page (REA): `SSRandomIntBetween(Int32, Int32) -> Int32`,
`SSRandomFloatBetween(CGFloat, CGFloat) -> CGFloat`, `SSRandomPointForSizeWithinRect(NSSize, NSRect) -> NSPoint`,
`SSCenteredRectInRect(NSRect, NSRect) -> NSRect`. The official pages give no further detail on the
order of the two rectangles or on the range rules. The choices made here are recorded in the code and in the
decisions below.

## Decisions and assumptions

- The random integer range is closed on both ends and the order of the bounds does not matter. The float range follows
  the same rule.
- `SSCenteredRectInRect` takes the rectangle to centre first and the container second, and keeps the first rectangle's
  size.
- `ScreenSaverView::start_animation` does nothing when already animating. The first frame comes one interval after the
  start. The reference notes that `startAnimation` and `stopAnimation` do not start or stop drawing at once, and that
  `animateOneFrame` can still run after a stop. `animate_one_frame` therefore ignores the timer.
- The catch-up cap of 4 frames is a BifrOSt choice, not an Apple value.

## Evidence

- REA evidence of the rendered official page: `ev_a115b6cf856edf0d27b365ece986ed09269d9736665f16a246f48238ff73efd0`
  (https://developer.apple.com/documentation/screensaver).
- DocC: the `screensaver` collection, the `ScreenSaverView` and `ScreenSaverDefaults` symbol pages, and the four utility
  pages (`tools/parity/docc.py`). The utility pages carry only one-line abstracts.

## Matrix rows closed

Status is `partial` for each row, because the plug-in host and the preview sheet are still missing.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Screen Saver | ScreenSaverView (plug-in view and animation lifecycle) | partial | springboard/ui/src/screensaver.rs |
| Screen Saver | Preference storage for a screen saver module | partial | springboard/sdk/src/screen_saver_defaults.rs |
| Screen Saver | Random value and rectangle helpers | partial | springboard/ui/src/screensaver.rs |

## Still open

- No `.saver` bundle loader and no engine that runs the view.
- No preview thumbnail and no configuration sheet in Settings.
- The backing store type and the gamma fade have no compositor hook.
- Idle state is not exposed to apps, and apps cannot hold a sleep inhibitor.
