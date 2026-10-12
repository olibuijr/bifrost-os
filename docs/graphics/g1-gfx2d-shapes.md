# G1 gfx2d-shapes: the SDF shape pipeline

Owner: wave G1 agent `gfx2d-shapes`. Contract: `docs/graphics-architecture.md`, "bifrost-2d".

## What is implemented

- `springboard/gfx2d/src/shapes.rs`
  - `Shapes::new(device, target)` builds the render pipeline `shape.vert` / `shape.frag` (premultiplied over, triangle
    strip, one texture at binding 0, one sampler at binding 1, 8 bytes of push constants) and a linear sampler.
  - `Shapes::draw(enc, instances, first, count, target, texture)` records one draw of `count` instances. The caller
    owns the buffer and the encoder. `texture` is bound at binding 0 even when no instance samples it.
  - Pure helpers, host-tested: `unpack` (packed premultiplied u32 to straight RGBA 0..1), `margin` and `quad`
    (the vertex fringe), `fill`, `stroke`, `shadow`, `gradient` (one instance per pair of stops), `image`, `mask`,
    `clip_words`, `op_instances` (maps `Op`s to instances, with opacity), `encode` (little-endian vertex bytes),
    `push_constants`, `vertex_layout`, `kind_of`.
- `springboard/gpu/shaders/shape.vert` and `shape.frag`: the instanced quad and the SDF fragment shader.
- `springboard/gpu/src/lib.rs`: one line, `pub use ash;`, so `gfx2d` can name `vk::Format` for the vertex layout
  without a Cargo change (see changes outside the file list).

## Instance layout (128 bytes, eight vec4 attributes, locations 0..7)

| attribute | rect | params | color | color2 | xform | xform_t | clip | extra |
|---|---|---|---|---|---|---|---|---|
| meaning | x, y, w, h (local px) | radius, width / shadow sigma / gradient t0, kind, gradient t1 | straight RGBA; gradient colour at t0; image alpha in `.a`; mask tint | gradient colour at t1; image and mask uv (u0, v0, u1, v1) | a, b, c, d | tx, ty; gradient start x, y | device-px clip x, y, w, h (w = 0: none; an empty clip draws nothing) | gradient end x, y; clip corner radius; gradient flags |

Kinds: 0 fill, 1 stroke (|d| band), 2 shadow (`0.5 * erfc(d / (sigma sqrt 2))`, sigma = blur / 2, offset applied to
the rect, quad grown by 3 sigma), 3 linear gradient segment, 4 image, 5 mask (red channel of the texture).

Gradient flags: bit 0 pads before t0, bit 1 pads after t1. A gradient with n stops is up to n-1 instances, one per
pair with positive width; pads are on the first and last instance only. Colours between stops interpolate straight
RGBA, and the shader premultiplies.

## Shader behaviour

- Coverage is 1 px analytic antialiasing: `clamp(0.5 - d / fwidth(d), 0, 1)`, so one device pixel at any transform.
- The quad is the rect grown by `1 / min_axis_scale` (one device pixel), plus 3 sigma for shadows.
- The clip is a rounded rectangle in device pixels from `gl_FragCoord`, multiplied into the coverage.
- erf is the Abramowitz and Stegun 7.1.26 approximation (error below 1.5e-7).
- Output is premultiplied RGBA.
- Image and mask sample `texture(sampler2D(u_tex, u_samp), uv)`, with uv interpolated over the rect from `color2`.
  The sampler is clamp-to-edge.

## Mapping to the contract

- Canvas `fill_rect`, `fill_rrect`: kind 0. `stroke_rrect`: kind 1. `shadow`: kind 2. `linear_gradient`: kind 3.
  `image`, `image_tinted`: kinds 4 and 5. `push_clip`: the renderer keeps the innermost rounded clip in device pixels
  and passes it as `Clip`. `push_transform`: the renderer passes the current `Transform2D`. `push_opacity`: the
  renderer passes the product of opacities, applied by `op_instances`.
- `Op` coordinates are the local coordinates of the recorded transform state. Gradient `start` and `end` use the
  same coordinates as `r`.

## Changes outside the file list

- `Instance` grew from 112 to 128 bytes with the `extra` field (gradient end, clip radius, flags). The contract's
  "112 bytes" is out of date; the layout is documented above.
- `bifrost_gpu` re-exports `ash` (one line in `gpu/src/lib.rs`).

## What is left (todo_left)

- Nested clips are not intersected here. The renderer must pass one clip per instance (innermost clip only), or split
  the draw when the clip changes.
- Gradients are exact for any affine transform, since the gradient axis is in local space. Hard stops at the same
  position produce a double-composited line of pixels where two instances meet.
- Shadows use the SDF Gaussian approximation of the rounded rect, not a true blur of the shape. Large blurs look
  correct; very small radii with large blurs are approximate.
- The renderer (`gfx2d-renderer`) must call `op_instances`, build the buffer with `encode`, and batch by kind and
  texture.
- Nothing has run on a GPU yet. The gpu crate is still skeleton (`gpu-pipeline`, `gpu-device`), so
  `new_render_pipeline` and `draw` are untested.

## How to verify

Host (no GPU needed), from `springboard/`:

    CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo test --offline -p bifrost-2d shapes

Shader compile (needs glslc, present on Titan and Midget):

    glslc --target-env=vulkan1.3 -O gpu/shaders/shape.vert -o /tmp/shape_v.spv
    glslc --target-env=vulkan1.3 -O gpu/shaders/shape.frag -o /tmp/shape_f.spv

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4), once the renderer and window server are wired: draw a window with a
rounded fill, a 1 px stroke, a shadow, a two-stop and a three-stop gradient, and a clipped image. Check edges at 1x
and 1.5625x scale for a one-pixel antialiased fringe, the shadow falloff against a flat background, and that the
clip cuts the corners of the image. Take a `bifrost pc midget screen` capture and compare it with the CPU raster
(`bifrost_render::Surface`) of the same display list.
