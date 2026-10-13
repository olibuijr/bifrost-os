# Core Image parity: pure Rust pixel models

Status after the second pass: still `partial`. The first pass added three render-crate models (colour,
lookup and gradient). The second pass, on branch `parity/team-j-core-image`, added the models below and an
SDK layer so apps can reach them. Photos does not use the new models, and nothing is verified on Midget yet.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/render/src/colorfx.rs` | CIExposureAdjust, CIColorControls (saturation, contrast), CIHueAdjust, CITemperatureAndTint, CISepiaTone, CIColorInvert, CIColorPosterize, CIVignette | Per-pixel functions on unpremultiplied RGB in 0..=1: `exposure`, `saturation`, `contrast`, `hue_rotate`, `temperature_tint`, `sepia`, `invert`, `posterize`, `vignette`, `vignette_factor`. `map_pixel` applies any of them to a packed pixel and keeps alpha. |
| `springboard/render/src/colorcube.rs` | CIColorCube | `ColorCube`: an N-sample (2 to 64) 3D table, red fastest, with trilinear `lookup`. `identity` builds the neutral table. |
| `springboard/render/src/gradient.rs` | CILinearGradient, CIRadialGradient | `Ramp`: colour stops with clamped `color_at`. `linear_t` and `radial_t` give the offset of a point. |

Each module has its own `#[cfg(test)] mod tests` (8, 3 and 6 tests respectively).

## Choices

- Pure functions, no allocation in the pixel paths except `ColorCube::identity` and `Ramp::new`.
- The temperature and tint model is a gain approximation. It is not a white-point conversion, so it
  does not match the reference output of the Apple filter.
- The packed pixel layout in `map_pixel` is red in the low byte, as in the Photos filters.
- Luma weights are Rec. 601 (0.299, 0.587, 0.114), the same as `springboard/src/shell/photos/edit.rs`.
- The modules are not wired into the shell or the SDK. Wiring is the next step, and it needs an SDK
  type and a decision on whether Photos moves onto `colorfx`.

## Official sources

- Framework overview: https://developer.apple.com/documentation/coreimage (fetched with
  `docc.py coreimage --depth 1`, which lists the Filter Catalog groups: Color Adjustment, Color Effect,
  Gradient, Generator and Blur).
- REA evidence of the rendered overview page: `ev_42258aba046578ec62c49e46424c6ad6f5c661fa5c10d547269f74d10d6d7468`.

## Second pass: effect models and SDK wrappers

| Module | Mirrors (reference names) | What it does |
|---|---|---|
| `springboard/render/src/color_curve.rs` | vibrance, CIToneCurve | `vibrance` boosts the less saturated colours more. `ToneCurve` is a five-point piecewise linear curve, with `identity`, `new` (validated) and `apply`. |
| `springboard/render/src/photo_effects.rs` | CIPhotoEffect* (chrome, fade, instant, mono, noir, process, tonal, transfer), CIFalseColor | `PhotoLook` names the looks, `apply` maps a colour through a short `colorfx` chain, `false_colour` mixes two colours by luma. |
| `springboard/render/src/feature_detect.rs` | rectangle case of CIDetector | `bright_regions` finds 4-connected regions above a luma threshold with their bounding boxes; `filled_rectangles` keeps the ones that fill at least a given share of their box. |
| `springboard/render/src/depth_blur.rs` | depth-of-field blur, disparity | `depth_to_disparity` and `disparity_to_depth`, `blur_weights` (distance from a focus depth as a mask), and `depth_blur`, which runs `blur_filters::masked_variable_blur` with that mask. |
| `springboard/render/src/displacement_filters.rs` | CIDisplacementDistortion | Moves each pixel by the red and green values of a map, nearest sampling, clamped at the edge. |
| `springboard/render/src/edge_tone_filters.rs` | CIEdges, CIHighlightShadowAdjust | `edges` (Sobel magnitude of the luma as grey), `highlight_shadow` (gain on bright and dark tones; defaults are the identity). |
| `springboard/sdk/src/image_effects.rs` | app API | Wrappers: `vibrance`, `tone_curve`, `photo_look`, `false_colour`, `highlight_shadow`, `edges`, `displacement`, `depth_blur`, `rectangles`. |

Each new render module has a `#[cfg(test)] mod tests` block, and the SDK module has its own tests.

Existing modules the matrix now cites: `springboard/render/src/blur_filters.rs` has gaussian, box, motion, zoom,
disc, bokeh and masked variable blur, and `springboard/sdk/src/image_blur.rs` and `image_filter.rs` already
expose them to apps. The earlier matrix text called these missing; it was out of date.

## Choices (second pass)

- Approximations are named as such in the module docs. The photo looks and the highlight and shadow curves are
  not the reference curves. The displacement scale maps map values 0 and 255 to `-scale / 2` and `+scale / 2`.
- Rectangle detection is a bright-region model with a fill test. It is not a perspective quadrilateral detector.
- Depth blur takes depth from the app. It does not estimate depth, and it reuses the existing masked blur.
- The SDK wrappers copy nothing: they call the render functions on the same `Image` type.

## Remaining gaps


- No lazy image graph (CIImage or CIContext equivalent) and no render destination for apps.
- No custom kernel or custom filter extension point for apps.
- Colour: the temperature and tint model is a gain approximation; Photos uses brightness and contrast only.
- Feature detection: axis-aligned rectangles only; no perspective quadrilaterals, faces, text or barcodes, and no
  barcode or QR generation.
- Depth: no depth estimation; the app supplies the depth map. No lens shape.
- Missing filters: comic, spot light, depth of field, circle splash, torus lens, and the perspective, triangle and
  parallelogram tiles.
- Colour lookup: palettize is missing; the colour cube, colour map and Lab modules are models only.
- Image input: JPEG and PNG only. No RAW, no HEIF, no colour-managed decoding for apps.
- Built-in filter catalogue: 28 names, not the full reference catalogue.

## Verification

Not built or tested in this pass: the second pass only wrote the code. The lead builds the render and SDK
crates, runs the focused tests (`springboard/render`, `springboard/sdk`), and then tests on Midget before any
row becomes `parity`.
