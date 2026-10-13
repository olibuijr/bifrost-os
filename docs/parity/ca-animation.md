# Animation model (Core Animation parity slice)

Status: model, engine and view wiring done; nothing here has been run on Midget. The host build and tests are the
lead's to run (this slice was written without compiling).

## Code

- `springboard/ui/src/anim_value.rs`: `AnimValue` (scalar, point, size, rect, colour, transform), `Animatable`,
  component access (`position.x`, `frame.size.width`, `transform.rotation.z`, ...), `ValueFunction`.
- `springboard/ui/src/keypath_anim.rs`: `KeyAnimation`, `Media`, `TimingFunction`, `MotionPath`, `AnimationSet`.
- `springboard/ui/src/anim_transaction.rs`: `TransactionStack`.
- `springboard/ui/src/anim.rs`: curves and springs; `Animation::velocity` and a retarget that carries velocity.
- `springboard/sdk/src/view/keyanim.rs`: `ViewState` (the `Animatable` of a view), `Window` methods.
- `springboard/sdk/src/view/animation.rs`: `Window::animate` opens a transaction scope; spring retargets keep velocity.

## Rows

| Row | Done | Midget |
|---|---|---|
| Implicit animation via transactions, media timing, timing functions | Nested begin/commit/flush, duration, timing function, disable actions, completion (waits for the animations made inside), named values, lock. `Media`: begin time, duration, speed, time offset, repeat count, repeat duration, autoreverse, fill mode. Named timing functions and control-point cubics. `set_value_animated` animates per key under the stack. | Draw a fade and a move with a custom duration; Reduce Motion on and off. |
| Basic property animation | String key paths, from/to/by (all combinations), additive, cumulative, removed on completion, add/remove by key, presentation versus model values. | Run an additive slide and a cumulative repeat on a real view. |
| Keyframe and group animations | Values, key times, per-segment timing, calculation modes linear, discrete, paced, cubic, cubic paced; path animation with rotation mode; groups on a shared clock; value functions rotate/scale/translate per axis. | Visual check of a path animation; transform keys need a view with a transform (views have none yet; the engine works on any `Animatable`). |
| Spring animation | Mass, stiffness, damping, initial velocity, settling duration; `Window::animate` and `set_value_animated` retarget springs with their velocity (`Animation::retarget`, `AnimationSet::retarget`). | Feel check of a retargeted spring. |
| Transaction completion and actions | Nesting, completion after committed and stopped, per-key values, disable actions. | None beyond the host tests. |

## Gaps

- Views have no transform, so transform keys animate only on other `Animatable` targets (the layer type).
- A property animated by `Window::animate` and by a key animation at once cancels the older track.
- Group children do not call their own delegates; a group's fill and removal apply to the whole group.
- Colours interpolate in plain RGBA.
- `springboard/layers` has its own typed animation model for the render server; it is not merged with this one (it has no string key paths, value functions, calculation modes or delegates). Merging belongs to the layers owners.
- Shell code (`springboard/src/shell`) animates with `render::motion` on its own draw path, not through SDK views, so there was nothing to migrate there.
