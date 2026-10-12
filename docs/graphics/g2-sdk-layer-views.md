# G2: sdk-layer-views (layer-backed views, layer scrolling, hover and press)

Owner: wave G2 agent `sdk-layer-views` (row 12 of `docs/graphics-waves.md`). Contract: `docs/graphics-architecture.md`,
"SDK" and "bifrost-layers". Branch `g2/sdk-layer-views`.

## What is implemented

### `springboard/sdk/src/layers.rs` (new, `pub mod layers` in `sdk/src/lib.rs`)

- `Path::{Shared, Direct}` and `Path::choose(requested, layer_protocol)`: direct only when `BIFROST_DIRECT=1` and the
  compositor offers `bifrost_layer_manager_v1`. Everything else is the shared-memory path.
- `LayerBatch`: the changes of one frame. Each change keeps the implicit animation in force when it was recorded.
  `animate(duration, curve, f)` sets the implicit animation for the block and restores the outer setting afterwards
  (nested blocks work). `into_transactions(first_seq)` splits the frame into `bifrost_layers::Transaction`s wherever the
  implicit setting changes, because a transaction carries one setting.
- `LayerTree`: the client side of the layer tree.
  - `sync(view, parent, frame, wants)` is `wants_layer`: creates the view's layer on first use (`Create` plus `Insert`
    under the parent or the root, index clamped to the end), updates its frame, destroys it when `wants` turns false.
  - `set_frame` writes `Bounds` (origin 0,0, frame size) and `Position` (frame centre, anchor 0.5). `set_opacity`,
    `set_background` and `set_highlight(view, Highlight, tint)` are property changes. Each one compares with the last
    value it sent and records nothing when unchanged.
  - `Highlight::{Normal, Hover, Pressed}`: the tint's alpha is 0, 0.06 or 0.12.
  - `animate(duration, curve, |t| ...)` is the animation block for the tree.
  - `flush()` ends the frame: returns the transactions with sequence numbers after the last one sent, empty when there
    is nothing to send or on the shared path.
  - Layer ids start at 2; `ROOT` (1) is the frame loop's root (`wl::frame::frame_transaction`).

### `springboard/sdk/src/view/scroll.rs` (pure model appended; `ScrollView` unchanged)

- `VelocityTracker`: finger samples; `velocity(now)` is the displacement over the last 100 ms (zero with fewer than two
  samples in the window, or when the finger has been still).
- `rubber(x, dim)` = `(1 - 1 / (x * 0.55 / dim + 1)) * dim`, and `displayed_offset(finger, max, dim)`: the content
  follows the finger inside the range and is rubber-banded past either edge (continuous at the edge).
- `ScrollPhysics`: `drag(t_ms, delta)`, `offset()`, `set_offset()`, `release(now_ms) -> Option<Motion>`.
  - A fling (speed of at least 20 pt/s) gives `Motion::Fling(Fling)`: exponential decay at the reference platform's
    normal rate (0.998 per ms), `offset_at(t)` clamped to the range, duration `ln(v / 20) / 2.002`.
  - A release past an edge gives `Motion::Bounce { from, to, velocity }`, a spring back to the edge.
  - Otherwise `None`.
- `motion_animation(motion, id, width, height) -> bifrost_layers::Animation`: a `Keyframe` on `Bounds` sampled every
  display frame (1/60 s, the last sample at the exact end) for a fling, or a `Spring` for a bounce. Both use fill
  `Forwards`. The content layer's bounds origin is the scroll offset (`content_bounds`).

## Mapping to the contract

| Contract item | Where |
|---|---|
| A view with `wants_layer` owns one layer, transactions batched per frame | `layers.rs` `LayerTree::sync`, `flush`, `LayerBatch` |
| `animate(duration, curve, \|\| ...)` sets the transaction's implicit animation | `LayerTree::animate`, `LayerBatch::animate`, `into_transactions` |
| Scroll is a layer bounds-origin change, the server runs the deceleration | `scroll.rs` `content_bounds`, `motion_animation` (Keyframe on `Bounds`) |
| Velocity from the last 100 ms of axis events | `scroll.rs` `VelocityTracker` |
| Rubber-banding at the edges | `scroll.rs` `rubber`, `displayed_offset`, `Motion::Bounce` |
| Hover and press are property changes | `layers.rs` `set_highlight`, `set_background` |
| Shared-memory path falls back to redrawing | `layers.rs` `Path::Shared` (no records, `sync` returns `None`); `ScrollView` untouched |

## Tests

- `layers.rs`: 13 tests. The shared path records nothing; the path truth table; the first sync's exact changes; an
  unchanged frame records nothing; removal; ids and parents; `animate` applies only inside its block; nested blocks
  restore the outer timing; hover and press alpha (0.06, 0.12) and the repeat guard; highlight strengths; sequence
  numbers across frames; grouping by implicit setting; ids never reused.
- `scroll.rs` (`physics_tests`): 13 tests. Velocity over the window (300 pt/s hand-worked from 10 to 40 pt in 100 ms);
  still finger gives zero; old samples drop; rubber band value 43.137 at x=100, dim=200; one-to-one inside the range;
  rubber past the top (-35.484); fling duration 1.95405 s and end 589.509 from offset 100 at 1000 pt/s; fling clamped
  at the content end; a slow release does not fling; a release past the edge springs back (-21.568 to 0); the
  keyframed animation has 119 samples and ends at 589.509; the spring's initial velocity; a new drag after a fling
  starts from the set offset.
- Verified: `CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2
  -p bifrost-app --tests` passes (exit 0, no errors). `python3 tools/check-names.py` passes. The tests were not run
  (this round allows only `cargo check`). Run them with `cargo test --offline -p bifrost-app --lib layers` and
  `... --lib physics_tests`.
- Warnings: the `scroll.rs` physics items are `dead_code` until a view uses them (see todo_left).

## Verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

This module is host-only until the direct runtime and the window server are wired, so Midget checks come after those
rows land:

1. Start the session with `bifrost-wm` (`/etc/bifrost/compositor` = `bifrost-wm`) or labwc with `BIFROST_DIRECT=1`.
   Confirm `BIFROST_DIRECT=1` selects `Path::Direct` only when the compositor advertises `bifrost_layer_manager_v1`
   (`bifrost pc midget state` shows the globals in the log).
2. A layer-backed view that changes frame inside `animate` should glide: `bifrost pc midget trace on`, move the view,
   `bifrost pc midget trace off`, and confirm the window server ran the animation (the `presented` events carry the
   transaction sequence numbers, and the app issued no redraw during the motion).
3. Scroll a long list with a fling: the content layer's bounds origin changes once per release, the decay runs in the
   window server, and the app's frame counter stays flat during the glide. `tools/bench.py --midget` reports the
   animation frame drops; the budget is 60 fps with no dropped frames.
4. Hover a row: the background change is a property change (no app redraw in the trace).

## What is left (todo_left)

- Wiring: `ScrollView` does not use `ScrollPhysics` yet. The direct path needs a `LayerTree` owned by the window (or
  the runtime), a content layer per scroll view, drag and release fed from the axis and pointer events (with 120-unit
  axis values converted to points), and the `motion_animation` batch sent through the layer context. The shared path
  keeps `Scroller`. This needs `sdk-direct-runtime` (the loop) and the layer context from `sdk-wl-frame`.
- `ViewBase` has no `wants_layer` flag and `View` has no layer hook. `LayerTree::sync` takes the flag as an argument
  so no view trait change is needed; the window should call it with each view's `wants` and frame after layout.
- Frame-level flush: nothing calls `LayerTree::flush` yet. The frame loop (`wl::frame`) must send each returned
  transaction with `bifrost_layer_context_v1.commit` and map the `presented` sequence numbers to the pacer.
- The fling keyframes are sampled, not evaluated by the server's spring solver. The window server's `Tree::presentation`
  evaluates the keyframes linearly between samples; the sampling step is one display frame.
- No intersection with a scroll boundary for nested scroll views, no momentum transfer between nested views, and no
  wheel input (the proto still delivers only Down, Move, Up and Cancel, per `scroll.rs`).
- Border and material changes are not animated (`Value` has no kind for them, per `g1-layers-animation.md`).
- `ScrollPhysics::release` does not bounce after a fling reaches an edge; the fling is clamped there. A spring from the
  edge after a fling would match the reference behaviour more closely.
- Hover and press colours are passed in by the caller; the named colour tokens belong to the shell's `ui.rs`, and the
  views that use `set_highlight` must take their tint from there.
- The `dead_code` warnings on the physics items disappear when the scroll view uses them.
