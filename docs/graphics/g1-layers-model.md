# G1: layers model (`bifrost-layers`: structure, apply, transactions)

Owner: `layers-model`. Contract: `docs/graphics-architecture.md`, "bifrost-layers". Branch `g1/layers-model`.

## What is implemented

- `tree.rs` `Tree::apply(&Transaction, now)`: the whole transaction runs on a scratch copy of the tree
  (`self.clone()`). The copy replaces `self` and takes the transaction's `seq` only when every change was accepted.
  A refused transaction leaves the tree, its `seq` and its running animations untouched.
- Structure: `Create` (rejects duplicates and `LayerId(0)`), `Destroy` (recursive: the subtree, the animations on those
  layers, the root reference and the superlayer's sublayer entry all go), `SetRoot`, `Insert` (detaches the child from
  its old superlayer, clamps the index, rejects cycles: the child is the parent or one of the parent's ancestors),
  `RemoveFromSuperlayer` (keeps the layer, no-op when it has no superlayer).
- Property changes: `Set` takes a `Value` of the property's kind (`Layer::value` / `Layer::set_value`, mapping:
  Bounds→Rect, Position/AnchorPoint→Point, Transform→Transform, Opacity/CornerRadius/ZPosition→Scalar,
  BackgroundColor→Color, Shadow→Shadow). Non-animatable keys (Hidden, MasksToBounds, Contents, ContentsGravity) and
  mismatched kinds are refused with `BadValue`. The dedicated changes (`SetHidden`, `SetMasksToBounds`, `SetBorder`,
  `SetMaterial`, `SetContents`, `SetContentsGravity`) write their fields directly.
- Implicit animations: when `tx.implicit = Some((duration, function))`, an animatable `Set` (and `SetShadow`) adds a
  `Basic` animation with `from` = the value the layer presents at `now` and `to` = the new value, `begin_time = now`,
  `duration`, `function`, and id `IMPLICIT_ID_BASE + n` (n counts up per tree). The model holds the new value at once.
  The new animation supersedes the layer's running top-level animations on the same key. No animation is added when
  the presented value already equals the new one.
- Explicit animations: `AddAnimation` checks every animated value (keys animatable, values of the property's kind,
  groups recursively), begins at `timing.begin_time` or `now`, and replaces a running entry with the same layer and id.
  `RemoveAnimation` (unknown id is a no-op) and `RemoveAllAnimations` remove entries.
- `layer.rs`: `Layer::value`, `Layer::set_value` (new, typed access for the two functions above).
- `transaction.rs`: no behaviour change; tests added.

## Mapping to the contract

| Contract item | Where |
|---|---|
| `Tree::apply(&Transaction)` atomic, validated first | `tree.rs` `apply` (scratch copy) |
| create/destroy recursive, set root, insert/remove with superlayer bookkeeping | `tree.rs` `apply_change`, `destroy`, `insert` |
| property sets with Value kinds per Property | `layer.rs` `value`/`set_value`, `tree.rs` `set_animated` |
| implicit animations from the presentation value at `now` | `tree.rs` `set_animated`, `presented_value` |
| add/remove animations as Running entries | `tree.rs` `apply_change` (AddAnimation, RemoveAnimation, RemoveAllAnimations) |
| `seq` | `tree.rs` `apply` |

Decisions beyond the contract text:

- `SetShadow` animates implicitly like `Set` (Shadow is an animatable property). Border and material do not.
- `ApplyError` gained `ZeroId` and `BadValue { layer, key }`. `Tree` gained a private `implicit_seq` field (the
  struct cannot be built with a literal outside the crate; `Tree::new()` and `Default` are unchanged).
- Ids at or above `IMPLICIT_ID_BASE` (0x8000_0000) are reserved for implicit animations.

## Depends on `layers-animation` (not done here)

`apply` reads the presented value through `presented_value`, which calls `MediaTiming::progress`, `Value::lerp`,
`TimingFunction::eval`, `Spring::progress` and `Spring::settling_duration`. Those are todo!s on this branch until
`layers-animation` lands, so a test that retargets a layer while an animation of the same key still runs would panic
here (no such test is in this branch). The keyframe segment and spring sampling in `tree.rs` (`sample`,
`keyframe_value`) duplicate what `Tree::presentation` will do; when both land, `presentation` should call the same
`sample` so the two cannot drift. Group children use their own timing with the group's begin time (the group's own
timing is not applied).

## Tests

- `tree.rs`: 24 tests (structure, atomic refusals, cycles, recursive destroy, implicit and explicit sets, dedicated
  setters, animation add/replace/remove, implicit ids). Expected values are worked out by hand from the chain
  1 > 2 > 3 used in the tests.
- `layer.rs`: 8 tests (frame, superlayer transform about the anchor, value kinds, defaults, animatable keys).
- `transaction.rs`: 6 tests (constructors, push, equality, implicit settings).
- Run: `cargo check --offline -p bifrost-layers --tests` passes with no warnings. The tests were not executed in this
  round (the rules for this round allow only `cargo check`); run `cargo test -p bifrost-layers` once the animation and
  codec modules are in, since the implicit-animation path needs them.

## Verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

This crate has no GPU code, so Midget is only involved once the window server and the SDK use it:

1. `cargo test --offline -p bifrost-layers` on the host (after the animation and codec owners merge).
2. When `bifrost-wm` consumes `Tree::apply` (`render_server/layers.rs`), commit a transaction with an implicit opacity
   change from the SDK path and check the animation runs at 60 fps with `bifrost pc midget trace on`, then
   `tools/bench.py --midget` for the frame timings.

## What is left (todo_left)

- Performance: `apply` clones the whole tree per transaction. Fine for hundreds of layers; a validation-only pass or
  copy-on-write sublayer lists is the next step if transactions show up in profiles.
- Presentation sampling shared with `Tree::presentation` (see above).
- Group timing is not applied to children; no validation of animation timing values (negative durations, NaN).
- Implicit animation for `Border`, `Material`, `Contents`, `ContentsGravity` (not animatable through `Value` today).
