# G1: layers-animation (timing, animations, presentation)

Owner: wave G1 agent `layers-animation`. Contract: `docs/graphics-architecture.md`, "bifrost-layers".
Files: `springboard/layers/src/timing.rs`, `springboard/layers/src/animation.rs`, and the presentation side of
`springboard/layers/src/tree.rs` (`presentation`, `take_finished`, `animating`, `damage`).

## What is implemented

### Timing functions (`timing.rs`)

- `TimingFunction::eval(t)`: linear is the identity; every Bézier solves `x(s) = t` with Newton from `s = t` (up to
  8 steps, staying in [0, 1]) and falls back to bisection (100 steps at most) when Newton leaves the interval or
  does not converge. The result is `y(s)`. Endpoints are exact (`eval(0) == 0`, `eval(1) == 1`); NaN is 0 and input is
  clamped to 0..1.
- Named curves use the control points documented on the enum (`EaseIn`, `EaseOut`, `EaseInEaseOut`, `Default`).

### Values (`animation.rs`)

- `Value::lerp` per kind: scalar, point, rect, colour (straight RGBA), shadow (colour, opacity, radius, offset) and
  transform. The transform lerp is component-wise (six affine entries), which is what the contract allows. Written as
  `a * (1 - t) + b * t` so that `t = 0` and `t = 1` return the endpoints exactly. Mismatched kinds keep `self` until
  `t >= 1`.
- `Value::added` (component-wise sum) is used by additive animations.
- `model_value` and `set_value` map `Property` to a `Value` kind: bounds, position, anchor point, transform, opacity,
  corner radius, z position, background colour and shadow. Border, material and the non-animatable properties have no
  value kind and are skipped.

### Media timing (`MediaTiming`)

- Local time is `(t - begin) * speed + time_offset`. Before `begin` the animation is not active.
- One cycle is `duration`, or `2 * duration` with autoreverse (forward leg, then back). `repeat_count` counts cycles:
  0, negative or NaN plays once; a fraction plays a partial last cycle; infinity repeats forever.
- `progress(begin, t)` returns the linear progress within the cycle (reversed on the back leg), or `None` when
  nothing shows. Fill modes: `Backwards` and `Both` show progress 0 before the start; `Forwards` and `Both` show
  the end of the last cycle after the active time (the start after a whole autoreverse cycle, otherwise the end).
  `Removed` shows nothing outside the active time.
- `ended(begin, t)` is true from the end of the active time on, so `take_finished` can report it.
- A negative speed plays backwards: it starts at `time_offset`, ends at local time 0 and is not active while the local
  time is above its total. A speed of 0 freezes the animation at `time_offset`. A zero duration is instant.

### Springs (`Spring`)

- `progress(t)` solves m x'' + c x' + k x = 0 with x(0) = -1 and x'(0) = initial velocity, and returns 1 + x. Three
  closed forms: under-damped (zeta < 1), critical (|zeta - 1| < 1e-9) and over-damped (zeta > 1). A degenerate spring
  (mass or stiffness not positive) jumps to 1 after 0.
- `settling_duration()` is the last time the response is outside 0.1% of the target. It bounds the envelope to find a
  horizon (doubling from one time constant), scans that horizon with a sampling density set by the oscillation period,
  and bisects the last exit to 60 steps. An undamped spring returns infinity; a degenerate spring returns 0.
- A spring's duration comes from `settling_duration`, not from `MediaTiming::duration`. Its `repeat_count` and
  `autoreverses` are ignored. After settling, `Forwards` holds the exact target.

### Keyframes

- `keyframe_value` finds the segment that contains the linear progress, takes the local fraction within it, and lerps
  with that segment's timing function (linear when missing). Key times that do not match the value count are spaced
  evenly. Before the first key the first value shows; after the last key, the last value.

### Groups

- A group's `speed` and `time_offset` scale the time its children see (`begin + (t - begin) * speed + offset`).
  A child begins at its own `begin_time` if it has one, otherwise at the group's begin. The group ends when all its
  children have. Its duration, repeat and fill are not applied; each child carries its own fill.

### Presentation (`tree.rs`)

- `presentation(t)` evaluates every running animation on each layer in the order the animations were added. A
  non-additive value replaces the value so far, an additive one adds to it. Hidden layers and their subtrees are
  left out.
- Painter's order: a layer, then its visible sublayers sorted by `z_position` (stable among equal values), each
  subtree after its parent.
- `to_root`: the root maps through its own `to_superlayer()` (so a root with position or transform is placed in the
  surface); each child composes its parent's `to_root` with its own `to_superlayer()`.
- `opacity` is the layer's own times every ancestor's.
- `clip`: the nearest ancestor with `masks_to_bounds` clips the layer. The clip is the bounding box, in root space, of
  that ancestor's bounds under its `to_root`, with its corner radius. A layer is not clipped by its own mask. Nested
  masks use the nearest one only, as the contract says; intersecting them is listed in todo_left.
- `take_finished(t)` removes the animations that ended by `t` and returns their ids in running order. An animation
  whose fill is not `Removed` writes its final value into the model first (additive values add to the model).
- `animating(t)` is true while any running animation has not ended, including ones that have not begun yet.
- `damage(t0, t1)`: the root-space rectangle that changed between the presentations at the two times. A layer whose
  presented state differs (geometry, opacity, clip, anything in `PresentedLayer`) contributes its area at both times,
  padded by its shadow and cut to its clip. Layers that appear or disappear contribute their area. The result is
  conservative (it may cover slightly more than the exact changed pixels) and `Rect::ZERO` when nothing changed.

## Mapping to the contract

| Contract item | Where |
|---|---|
| Timing functions (linear, ease in/out/in-out, cubic Bezier) | `TimingFunction::eval`, `control_points` |
| Basic, keyframe, spring, group animations | `Animation`, `contributions`, `ended_at` |
| Media timing: begin, speed, time offset, repeat, autoreverse, fill | `MediaTiming::progress`, `ended`, `phase_with` |
| `Tree::presentation(t)` | `tree.rs` `presentation`, `emit`, `present_layer` |
| `Tree::finished(t)` (`animation_finished`) | `take_finished` (the contract's name; the skeleton has `take_finished`) |
| `Tree::damage` | `tree.rs` `damage` |

## What is left (todo_left)

- `Tree::apply` (the transaction side, implicit animations and their `from` values) belongs to `layers-model` and is
  still `todo!`. Implicit animations need `apply` to start a `Basic` at the presentation value; the animation code
  already accepts `from: None` for model values.
- Nested masks: only the nearest mask clips, per the contract. The renderer or the model must intersect when two
  masks nest and both clip.
- `Border` and `Material` have no value kind, so they cannot be animated yet. Adding a kind is a contract change.
- `damage` ignores blur and border overhang beyond the shadow padding; material regions need their own padding in the
  renderer.
- Group time mapping is a plain scale and offset; children are not re-timed relative to each other.

## Verification

Host (no GPU needed):

```
cd springboard && CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents \
  cargo check --offline -j 2 -p bifrost-layers --tests
```

The check passes. The unit tests (`timing`, `animation` and `tree::tests`) carry hand-computed values and an
independent RK4 integration as the spring oracle. They run with `cargo test -p bifrost-layers` on the host, which is
the lead's step, not this agent's.

On Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): this module has no GPU code. It is exercised once the render server
composes `Tree::presentation` (window server `render_server/layers.rs`). Check that a Dock magnify or a window open
(`MediaTiming` with the `Default` curve, and a spring for the bounce) keeps 60 fps while an app is busy, using
`tools/bench.py --midget`, and that `animating` keeps the frame clock running only while an animation is pending.
