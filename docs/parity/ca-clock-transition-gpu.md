# Frame clock, transitions, drawable layer (slice ca-clock-transition-gpu)

## Frame clock
- API: `media_time()`, `FrameLink` (callback, `paused`, `preferred_frame_rate_range`, `invalidate`), `Window::add_frame_link`, `remove_frame_link`, `frame_link_mut`, `set_display_refresh`.
- Paths: springboard/sdk/src/frame_clock.rs, springboard/sdk/src/view/animation.rs (`next_wake`, links), view/mod.rs (`Window::tick` fires links).
- Done: links run from the window tick at their resolved rate; `next_wake` uses the fastest link. Midget: check wake cadence with a 30 and a 120 link.

## Frame-rate range
- Path: springboard/ui/src/frame_rate.rs (`fixed`, `interval`, `Default`), re-exported as `bifrost_app::frame_rate`.
- Done: wired into the window tick through links. Midget: the shell host does not yet pass a real display rate (`set_display_refresh` defaults to 60).

## Transitions
- Path: springboard/ui/src/transition.rs (`Transition::fade/move_in/push/reveal`, `Subtype`, `Ease`, `range`, `reversed`, `parallax`, `Run`), re-exported as `bifrost_app::transition`.
- Migrated: springboard/src/shell/settings.rs (page slide) and springboard/src/files/nav.rs (`placement`). Removed Settings' private `ease` fn.
- Not migrated: the launch card cross-fade in shell/mod.rs (own timing). Midget: compare the push/pop slides with before.

## Drawable layer and offscreen renderer
- Paths: springboard/sdk/src/gpu_layer.rs (`DrawableLayer`, `next_drawable`, `present`, `take_presented`, `display_link`), springboard/sdk/src/offscreen.rs (`render_window`).
- Done: CPU-pixel pool with a host-side `take_presented`. Still needed: the shell/compositor (springboard/src/gpu/layer.rs) must call `take_presented` and upload it as a layer; not wired. Midget: verify.
