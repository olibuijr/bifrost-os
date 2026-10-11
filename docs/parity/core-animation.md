# Core Animation parity: pure models for keyframes, media timing and frame-rate ranges

Framework: Core Animation (https://developer.apple.com/documentation/quartzcore). Matrix row:
`docs/parity-matrix/data/core-animation.json`. Status after this round: still `partial` (three gaps moved to partial
models; no app-facing API yet).

## What was built

All three are pure std-only models in `bifrost-ui`, with no renderer or display access.

1. **Keyframes** in `springboard/ui/src/anim.rs` (`Keyframes`, `KeyframeMode`). Mirrors the `values`, `keyTimes`,
   `timingFunctions` and `calculationMode` (linear and discrete) of `CAKeyframeAnimation`. Key times default to even
   spacing. Each segment can have its own `Curve`. Invalid keys (empty, mismatched counts, descending, NaN) return
   `None`. Before the first key time the first value is returned, and from the last key time on the last value.
2. **Media timing** in `springboard/ui/src/anim.rs` (`MediaTiming`). Mirrors the timing properties of
   `CAMediaTiming`: `beginTime`, `speed`, repeat count and `autoreverses`. `progress(now, start)` gives the local 0..=1
   input for a curve, or `None` before the animation begins. Odd cycles play backwards when reversing, and a finish
   after an even number of cycles returns to the start. An infinite repeat never finishes.
3. **Frame-rate range** in `springboard/ui/src/frame_rate.rs` (`FrameRateRange`). Mirrors `CAFrameRateRange`
   (`minimum`, `maximum`, `preferred`, `default`) and `CADisplayLink.preferredFrameRateRange`. `resolve(display_max)`
   returns the target rate: the preferred rate, limited to the range and the display. A zero preference targets the
   highest allowed rate. The default range resolves to `None`, meaning the system chooses.

The old `Animation::progress` now calls the shared `progress_of` helper, so its behaviour is unchanged.

## Official API names mirrored

- `CAKeyframeAnimation`: `values`, `keyTimes`, `timingFunctions`, `calculationMode` (linear, discrete). Doc
  https://developer.apple.com/documentation/quartzcore/cakeyframeanimation
- `CAMediaTiming`: timing properties (`beginTime`, `speed`, `repeatCount`, `autoreverses`). Doc
  https://developer.apple.com/documentation/quartzcore/camediatiming
- `CAFrameRateRange`: `init(minimum:maximum:preferred:)`, `default`. Doc
  https://developer.apple.com/documentation/quartzcore/caframeraterange
- `CADisplayLink.preferredFrameRateRange`. Doc https://developer.apple.com/documentation/quartzcore/cadisplaylink

Official API shape was read with `docc.py` (quartzcore/cakeyframeanimation, quartzcore/caframeraterange,
quartzcore/cadisplaylink, quartzcore/caspringanimation). Keyframe calculation-mode values were not expanded beyond the
page title, so the modes modelled here are the two named in the matrix row (linear and discrete).

## REA evidence

- `ev_6ec360be3964ddc408ce9424d4360e85a54e8d830e4ea978de5ee8dd3b92255b`: REA passive inspection of
  https://developer.apple.com/documentation/quartzcore as rendered, listing the Animation, Animation Timing, Animation
  Groups, ProMotion and Particle Systems topics used here.
- `ev_7b05d4f65e159dad2ad9b422203d8d3c31411358dcb1aa5c37bd4b5f1825c75f`: the earlier landing-page evidence already in
  the matrix row (from the import, not re-run in this round).

## Not built, and why

- **Apps cannot reach any of the three yet.** The SDK's `Window::animate` (`springboard/sdk/src/view/animation.rs`) still
  diffs typed properties with `Timing`. Wiring `Keyframes`, `MediaTiming` and `FrameRateRange` into it, and adding a
  display-link object, is a larger SDK change and was left out of this pure-model round.
- Paced and cubic keyframe modes, path animation, tension, continuity and bias values, and group animations.
- Spring velocity carried through a retarget (the `Curve::Spring` variant would need a velocity field, and
  `springboard/sdk/src/view/animation.rs` matches on it).
- Layer tree, transforms, corner radius and shadows, shape and gradient layers, replicator and emitter layers, and
  Metal layers, as listed in the matrix.

## Verification

Unit tests in `springboard/ui/src/anim.rs` (`evenly_spaced_linear_keyframes_interpolate_between_neighbours`,
`explicit_key_times_place_the_keys_and_discrete_mode_holds_each_value`, `per_segment_timing_eases_only_that_segment`,
`a_zero_duration_keyframe_animation_jumps_to_its_last_value`, `keyframes_with_unusable_keys_are_refused`,
`a_repeated_key_time_never_divides_by_zero`, `media_timing_waits_for_its_begin_time_and_scales_with_speed`,
`repeats_wrap_each_cycle_and_then_finish`, `autoreverses_plays_odd_cycles_backwards_and_ends_at_the_start`,
`an_infinite_repeat_never_finishes_and_a_zero_duration_is_instant`) and in `springboard/ui/src/frame_rate.rs`
(`the_default_range_resolves_to_nothing` and six more). Not built or run in this round, per the stage rules; the
next stage builds and tests. `python3 tools/check-names.py` reports no hits.
