# Core Image parity: pure Rust pixel models

Status after this pass: still `partial`. Three render-crate modules now cover part of the framework's
colour, lookup and gradient features as pure std code. None of them is an SDK type yet, and Photos does
not use them.

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

## Remaining gaps

- No filter object, no chaining by name, and no SDK type for a lazily rendered image.
- Colour adjustment: vibrance, tone curve, gamma, and a white-point temperature conversion.
- Blur: motion, zoom, disc and bokeh blur; no radius or sigma API for apps.
- Photo effects: chrome, fade, instant, transfer and false colour; colour map, palettize and Lab conversion.
- Gradient generators: checkerboard and stripes. Barcode and QR generation.
- Feature detection on images (faces, rectangles, text, barcodes).
- Custom kernels and a custom filter extension point.
- Image input: JPEG and PNG only. No RAW, no HEIF, no colour-managed decoding for apps.
- Filtered frame output to a render destination.

## Verification

Not built or tested in this pass, as the task required. Build and test them in the next stage with the
focused crate tests (`springboard/render`). The owner name check (`python3 tools/check-names.py`) reports
no hits for the new `.rs` files.
