# Animation smoothness (slice `animation-smooth`, round 75)

Goal: the desktop runs smoothly on the lab PC (Intel GPU, 1080p to 1440p). Animations run on elapsed time from the
shared motion contract (`render/src/motion.rs`), skip frames under load without changing speed, and honour Reduce
Motion. No Apple names in code or comments.

## Audit (what was already time-based)

- Window open, close, minimise, restore and zoom: `src/shell/window_fx.rs` and `desk_fx_start` / `fx_tick` in
  `src/shell/desk.rs` run on `motion::Anim` with the clock `t / 1000`. Reduce Motion becomes a cross-fade in place.
  Close starts (`FxCall::Close`, desk.rs) and ends hidden.
- Mission Control and the desktop veil: `src/shell/overview.rs` (`OverviewEnter`, `PanelReveal` through `progress`).
- Launchpad: `src/shell/appgrid.rs` (`OverviewEnter` open and close, `PanelReveal` page settle).
- Dock: magnification `dock_mag` (`DockMagnify`), hover label (`CrossFade`), launch and attention bounces
  (`DockBounce`), autohide slide (`PanelReveal`) in `src/shell/desktop_dock.rs` and `src/shell/desk.rs`.
- Render loop: `ui/src/render_loop.rs` schedules the next frame from `now + interval`, so a late frame skips rather
  than queueing a burst. The shell's animation time is the presentation time of the frame being drawn
  (`src/bin/shell.rs`).

## Changes

| Commit area | Before | After |
|---|---|---|
| `src/shell/photos/viewer.rs`, `zoom.rs` | The rubber-band snap-back took a fixed quarter per `tick`, so its speed followed the frame rate and a stalled frame changed it. | `zoom::decay(v, dt_s)` retains a quarter per 1/60 s of elapsed time (`last_tick` clock in the viewer). Test: two half-frames equal one full frame. |
| `chrome/src/panels.rs` | `slide_offset` had its own ease-out cubic. | Uses `motion::ease(Curve::EaseOut, p)`, the same curve as `PanelReveal`. |
| `chrome/src/dock.rs` | Magnification eased on a private exponential even under Reduce Motion. | Snaps to the target when `motion::duration(Kind::DockMagnify)` is 0. |
| `sdk/src/lib.rs` (kit) | Apps had no path to the motion contract. | `pub use bifrost_render::motion;` in `bifrost_app::kit`. |
| `apps/music-desktop` (1.6.2) | Now Playing sheet: private 350 ms quartic from `shell::gestures`, ignored Reduce Motion. | `motion::Anim` with `Kind::PanelReveal` (`sheet_slide`), snaps under Reduce Motion. `SHEET_MS` removed. |

## Left as is (and why)

- Control Center settle and the app switcher use `shell::gestures::ease_out_quartic` with `SPRING_MS` (350 ms).
  Their tests pin the 350 ms timing values at fixed clock points, and changing the curve needs a build to re-derive
  them. Candidates for a later pass that moves them onto `Kind::PanelReveal` and updates those expectations.
- `Kind::MenuAppear` and `Kind::SheetSlide` have no callers. Menu bar dropdowns do not animate yet, so there is no
  menu transition to convert. The SDK sheet widget takes a caller `progress` and no app drives it with `SheetSlide`.
- `Kind::SheetSlide` is a spring; the music sheet uses `PanelReveal` (ease-out cubic) so it lands without overshoot.

## Still needs a check on Midget

- Window close and minimise at 1080p and 1440p with the Intel GPU: smooth, no skipped frames in the jank log.
- Photo viewer overscroll snap-back after a fast drag, then a stall: it should still settle in about the same time.
- Dock magnification and the Now Playing sheet with Reduce Motion on and off (the system Reduce Motion preference).
- The stale CHANGELOG line for the window close effect ("Close does not animate yet") is out of date; close does
  animate. Fix it with the next shell release.
