# G1 `wm-material`: live glass and window shadows

Owner: wave G1 agent `wm-material`. Contract: `docs/graphics-architecture.md`, "Window server",
`render_server/material.rs`.

## What is implemented

`springboard/wm/src/render_server/material.rs`

- `kawase_passes(radius)`: 1 pass reaches 2 px and each further pass doubles the reach, clamped to 1..6
  (0 and below, NaN and small radii give 1; 33 px and up give 6).
- `kawase_offset(radius, passes)`: the kernel offset in source texels, `radius / 2^passes`, at least 0.5. The
  offset is what spreads the radius over the levels.
- `window_shadow(rect, radius, active, scale)`: active is sigma 24 with offset (0, 16) and alpha 0.35, inactive
  is sigma 12 with offset (0, 6) and alpha 0.20. Sigma and offset scale with `scale`; the colour is black,
  premultiplied; `rect` and `radius` are passed through unchanged (already in output pixels).
- `SATURATION` = 1.8, the shell's glass value (`wm/src/glass.rs`).
- Pure helpers, all tested: `device_region` (outward whole-pixel rounding), `pyramid_sizes` (halve and round up,
  at least one texel), `shadow_bounds` (shadow rect moved by its offset, grown by 3 sigma, clipped to the target),
  and the push-block encodings `PushBlock` (88 bytes) and `ShadowPush` (48 bytes), checked byte by byte against
  the std430 offsets of the shaders.
- `Pipelines::new(device)`: two compute pipelines (`kawase_down.comp`, `kawase_up.comp`, one sampled texture,
  one sampler and one storage image each), one render pipeline (`fullscreen.vert` + `shadow.frag`, premultiplied
  over, no vertex input) and a linear sampler.
- `Pipelines::record_blur(cb, target, region, pyramid)`: records the whole blur into the caller's command buffer.
- `Pipelines::record_shadow(cb, target, spec)`: records the shadow as a scissored fullscreen triangle.
- `blur_pyramid(device, w, h, passes)`: allocates the levels T_1..T_n as `Rgba8Unorm`, `SAMPLED | STORAGE`.

Shaders (`springboard/gpu/shaders/`):

- `kawase_down.comp`: the dual-Kawase down step. Centre weight 4 and four diagonal taps at the offset, over 8.
- `kawase_up.comp`: the dual-Kawase up step (3x3 tent, weights 1 and 2, over 12). With `final_pass = 1` it also
  saturates, adds the tint (`tint + c * (1 - tint.a)`), computes the rounded-rect SDF coverage (1 px AA) and
  blends the result over the target texel with `mix`, so only pixels inside the rounded rect change.
- `shadow.frag`: the rounded-rect SDF at the offset position, turned into `0.5 * erfc(d / (sigma * sqrt 2))`
  with the Abramowitz and Stegun 7.1.26 erf approximation, times the premultiplied colour.

## How it maps to the contract

- The pass order in the contract is: copy the region behind the material into a half-size texture, down and up
  passes, then draw back clipped. Here the first down pass reads the target region through the sampler, so the
  copy is the first pass itself. The pyramid is T_k = region >> k, k = 1..n. Down passes write T_1..T_n; up
  passes write T_(n-1)..T_1; the final up step writes the target region at full resolution with the saturation,
  tint and rounded-rect clip. For n = 1 the final step reads T_1 directly.
- The clip and the tint happen in the compute pass, so the compose pass needs no render encoder for glass.
- Shadows are a render pass with `LoadAction::Load`, so they are drawn over what is already in the target.
- `compose.rs` owns the draw order and the calls; `DrawItem::material` and `DrawItem::shadow` carry the values
  this module consumes.

## Decisions and deviations

- Half resolution is used for the pyramid (contract). The shell's glass uses a quarter (`wm/src/glass.rs`
  `DIV = 4`); the contract wins.
- The GPU API calls use the binding plan below. Texture bindings and sampler bindings have separate numbers in
  set 0: 0 sampled texture, 1 sampler, 2 storage image. `gpu::Device` must create the descriptor set layout with
  those numbers (the gpu owners are still skeletons). If the gpu crate counts bindings differently, change the
  `set_*` calls here, not the shaders.
- Push constants are 88 bytes for the blur and 48 bytes for the shadow, under the 128-byte limit. vec4 members
  come first in both blocks so std430 needs no padding.
- The target must be `Rgba8Unorm`. BGRA targets would need a swizzle variant; the compose target is the place to
  choose the format.
- The blur reads and writes the target in separate dispatches. The final step reads the target texel it writes,
  which is safe because each invocation touches only its own texel, but the layout transition between the
  sampled use and the storage use relies on the gpu crate's layout tracking.

## Verification

Host, pure logic (run by the owner, not by this agent):

    cd springboard && cargo test --offline -p bifrost-wm render_server::material

The tests cover: passes at the boundaries (2, 2.5, 4, 5, 8, 9, 16, 17, 32, 33, 64), clamping and NaN, the offset
values, the active and inactive shadow values with scale 1, 1.5 and 2, outward region rounding, the pyramid sizes
(100x50 with 3 passes gives 50x25, 25x13, 13x7), shadow bounds with and without clipping, and the byte layout of
both push blocks.

Build check that passed: `CARGO_TARGET_DIR=.../springboard/target-agents cargo check --offline -j 2 -p bifrost-wm --tests`
(no warnings from `material.rs`). Name guard: `python3 tools/check-names.py` prints no hits.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4), once `bifrost-gpu` and the render server are live:

1. Build the fast profile and deploy with `tools/midget-push-shell.sh`, with `BIFROST_WM_RENDERER=vulkan`.
2. Open a settings window over a busy desktop wallpaper, and compare the blurred backdrop and its saturation with
   the GLES path (`BIFROST_WM_RENDERER` unset) in a screenshot.
3. Check the rounded corners of a glass panel at 17 px and 9 px radii with a zoomed screenshot.
4. Check active and inactive window shadows against the reference values (sigma 24 and 12).
5. Measure the material cost with `tools/bench.py --midget` (target: under 4 ms GPU for the whole frame, budget
   from `docs/graphics-architecture.md`) and add the row to `docs/perf-history.jsonl`.

## What is left (todo_left)

- `bifrost-gpu` is still a skeleton: `Device`, `CommandQueue::command_buffer`, `compute_encoder`, `render_encoder`,
  `set_*`, `dispatch`, `draw`, `Texture::replace_region` and `new_texture` all panic with `todo!`, so nothing here
  runs yet. The code is written against the API as declared.
- The shaders are compiled only when `glslc` is present (`gpu/build.rs`). They have not been compiled or run.
- `compose.rs` must call `record_blur` before the item it belongs to, and `record_shadow` before the window
  (both are in `DrawItem`). It also has to keep a pyramid per material size and reuse it between frames.
- A GPU test (behind `gpu-tests`) that compares the blur with a CPU reference of the same kernels is not written.
- A blur of a region that is partly outside the target is clipped by `device_region` only; the sampler's clamp to
  edge takes care of the taps, but there is no test for it.
- Blur quality (kernel offset per level, the reach against the reference look) is only set from the formulae; it
  needs a Midget comparison to tune.
