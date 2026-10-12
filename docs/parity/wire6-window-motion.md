# Wire 6: window and symbol motion (desktop shell)

Wires the motion models to the desktop shell: the window open, close, minimize and restore effects, the Dock
icon bounce while an app launches, and the symbol bounce of a Control Center toggle. The models are
`springboard/render/src/motion.rs` (curves, durations, `Anim`), `springboard/src/shell/window_fx.rs` (window
effects) and `springboard/ui/src/symbol_effect.rs` (symbol effects). No new model was written. The code is not
built in this wave (the lead builds once), and nothing here has run on Midget yet.

## What is now wired

| Feature | How a user reaches it | Code |
|---|---|---|
| Minimize to the Dock icon (scale and slide, fade over the last third) | Yellow traffic light, or the Window menu's Minimize; the window flies to its Dock tile | `desk.rs` `desk_minimize_window` → `desk_fx_start(FxCall::Minimize)`; `window_fx.rs` `Effect::Minimize` |
| Restore from the Dock (scale up from the tile) | Click a minimized window's Dock tile | `desk.rs` `desk_open` → `FxCall::Restore` |
| Open (scale 0.9 to 1 and fade in, spring curve) | Open any app from the Dock, Spotlight or the app grid | `desk.rs` `desk_open` → `FxCall::Open` |
| Close (scale 1 to 0.9 and fade out, 0.16 s) | Red traffic light, or File > Close Window (Cmd+W) | `desk.rs` `desk_close_app` → `FxCall::Close`; the closing window's last frame is kept in `Desk::ghosts` and drawn at its old depth until the effect ends |
| Dock icon bounce while an app launches | Click a closed app's Dock tile, or open it from Spotlight or the app grid; the tile hops for one `DockBounce` (0.7 s) once the window maps | `desk.rs` `desk_open` calls `dock_start_bounce`; `desktop_dock.rs` `dock_tick` keeps the bounce one hop after the window maps (it used to drop at once) |
| Symbol bounce on a Control Center toggle | Open the Control Center panel and tap Wi-Fi, Bluetooth or Focus; the glyph bounces once. The cluster's Airplane, Rotation Lock and Silent toggles start the same bounce through `activate_tile` | `control_center.rs` `ControlCenter::start_toggle_bounce`, `glyph_scale`; `cc_desk_event` and `activate_tile` call it; `draw_desk_cc` scales the glyph |
| Reduce Motion | Settings, Accessibility page, Reduce Motion switch | window effects become a cross-fade in place (`window_fx.rs`, `FxTracker::begin` skips Zoom); the dock bounce has duration 0 (`motion::duration`); the symbol bounce is not started (`start_toggle_bounce`) |

Minimize and restore already used `window_fx.rs` before this wave. This wave adds the close effect, the
launch bounce from every way of opening an app, and the symbol bounce.

## How to verify on Midget

1. Open Settings (`bifrost pc midget open-app Settings`), then take `bifrost pc midget screen /tmp/open.png`
   while it appears: the window should be smaller and fading in, then settle.
2. Click the red traffic light (or File > Close Window). Take a screenshot 80 ms after: the window should be about 95% size and half
   faded. Under Reduce Motion it should only fade.
3. Click the yellow light. The window should fly to its Dock tile and vanish; click the tile to bring it back.
4. Click the Terminal Dock tile while it is closed. The tile should hop once (about 0.7 s), then rest.
5. Swipe down from the top-right of the menu bar to open Control Center. Tap Wi-Fi with
   `bifrost pc midget tap-window ControlCenter X Y` (or the panel's own window coordinates). The Wi-Fi glyph
   should swell to about 1.15 and settle within 0.6 s. Repeat for Bluetooth and Focus.
6. Turn on Reduce Motion, then repeat steps 2 and 5. Expected: no scale, the close is a cross-fade, and the
   toggle glyph does not move.

## Host tests

- `springboard/src/shell/window_fx.rs` `a_closing_window_shrinks_and_fades_over_its_close_time`: at 0.08 s of a
  0.16 s ease-out close, progress 0.875, so alpha 0.125 and scale 0.9125; a utility window may close but not minimize.
- `springboard/src/shell/control_center.rs` `a_toggle_bounces_its_glyph_once_and_only_toggles_bounce`: a 0.6 s
  bounce at 0.3 s is 1 + 0.15 sin(pi/2) = 1.15; a non-toggle (Brightness) starts nothing; the bounce is over at 0.7 s.
- Existing `window_fx.rs` and `desktop_dock.rs` tests (`attention_bounce_hops_then_stops`, `reduce_motion_cross_fades_in_place`)
  still describe the same curves; `desk.rs` and `desktop_dock.rs` have no new test, because they need a Shell and the
  Wayland presenter.

## Evidence

Official pages were not re-read in this wave. The rows keep the evidence ids already on them:

- core-animation (`rea_evidence`): `ev_7b05d4f65e159dad2ad9b422203d8d3c31411358dcb1aa5c37bd4b5f1825c75f`
- symbols (`rea_evidence`): `ev_4228c223ea1ec28dfa7061baa7d6168c30cb9d1b86003c9d7dcf0cab875728f3`

## Remaining gaps

- The close ghost is drawn only on the Wayland presenter path (`desktop-wayland` with `SPRINGBOARD_WAYLAND`). In
  the host build no close effect runs.
- The Dock minimize setting `Funnel` still runs the scale and slide (the funnel warp needs a mesh the presenter
  cannot draw; recorded in `motion.rs` `MinimizeEffect`).
- The symbol bounce is on the desktop panel only. The mobile-style cluster in `draw_cc` does not bounce yet.
- Only the Bounce effect of the symbol model is used. Pulse, Appear, Disappear and the replace transitions are not wired.
- Keyframe and group animations (CAKeyframeAnimation, CAAnimationGroup) are not wired; the window effects are a
  single `Anim` per window.
- The Dock bounce of a launch started from Files opens the Files entry's tile, which may not be the tile that was
  clicked when Files maps to an external app.
- No build, no test run and no Midget run in this wave.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data) | BifrOSt path |
|---|---|---|
| core-animation | Basic property animation (CABasicAnimation, CAPropertyAnimation) | springboard/src/shell/window_fx.rs (open, close, minimize and restore scale and alpha through `motion::Anim`) |
| core-animation | Implicit animation via transactions (CATransaction, CAMediaTiming, timing functions) | springboard/src/shell/window_fx.rs and springboard/render/src/motion.rs (window effects use the motion curves) |
| symbols | Bounce and pulse effects | springboard/src/shell/control_center.rs (`start_toggle_bounce`, `glyph_scale`, desktop panel toggles) |
