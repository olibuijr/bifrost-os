# Wave 4: Core Image gap rows (pure Rust models)

Framework: Core Image, https://developer.apple.com/documentation/coreimage (macOS 10.11 and later).
Matrix data: `docs/parity-matrix/data/core-image.json`. This wave does not edit the matrix; the lead
imports the rows below.

## What was built

Three pure std models in `springboard/render/src/`. None is wired into an app or the shell yet, and none
has been built, run on the VM or run on Midget. Each module has host unit tests, with expected values
worked out by hand from the formulas.

- `blend.rs`: the twelve separable blend modes (`Mode`: Normal, Multiply, Screen, Overlay, Darken,
  Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion) and `composite`, which puts
  one premultiplied packed pixel over another under a mode (W3C compositing formulas). Pixels use the
  renderer's packing (red in the low byte, alpha on top).
- `blur_kernels.rs`: `motion_blur` (a line of samples at an angle), `zoom_blur` (samples along rays towards
  the centre) and `disc_blur` (a uniform average over a disc of whole-pixel offsets). Samples outside the
  image clamp to the edge. Each returns `None` when the buffer length is not `w * h`.
- `effect_chain.rs`: `Filter::named` builds one of nine named per-pixel effects (exposure, saturation,
  contrast, hueRotate, temperatureTint, sepia, invert, posterize, vignette) with defaults. Inputs are read
  and set by key (`get`, `set`, `input_keys`). `Chain` runs its filters in order over a packed buffer. The
  effects call the existing `colorfx` functions.

## Official API names mirrored

Mirrored as concepts only; no Apple identifiers appear in the code.

- Blend and composite: CIMultiplyBlendMode, CIScreenBlendMode, CIOverlayBlendMode, CISourceOverCompositing
  and the other composite and blend filters (Composite Operations collection).
- Blur: CIMotionBlur, CIZoomBlur, CIDiscBlur (Blur Filters collection).
- Filter objects: CIFilter (set inputs by key, chain filters), CIFilterGenerator (chain of filters).
- Colour effects used by the chain: CIExposureAdjust, CIHueAdjust, CIColorControls, CITemperatureAndTint,
  CISepiaTone, CIColorInvert, CIColorPosterize, CIVignette (already modelled in `colorfx.rs` by wave 1).

## REA evidence

- `ev_3f6a2488755d9e326cf62f04ba77ddb6e1cdfeaa73283049d55fd0711cbcd517`: rendered page
  https://developer.apple.com/documentation/coreimage ("Core Image is an image processing and analysis
  technology ... build complex effects by chaining filters"; Filter Catalog includes Blur Filters and
  Composite Operations).
- DocC index read with `python3 -I tools/parity/docc.py coreimage`.

## Matrix rows closed

| Framework | Feature (exactly as in data/core-image.json) | New status | BifrOSt path |
|---|---|---|---|
| Core Image | Composite operations and blend modes | partial | springboard/render/src/blend.rs |
| Core Image | Blur filters (gaussianBlur, boxBlur, motionBlur, zoomBlur, discBlur) | partial | springboard/render/src/blur_kernels.rs |
| Core Image | Filter objects and chaining (CIFilter) | partial | springboard/render/src/effect_chain.rs |

## Still open

- No app-facing SDK wrapper: `springboard/sdk/src/` has no filter or blend type yet.
- The gaussian blur with a sigma, and the disc and bokeh shapes beyond the uniform disc, are not modelled.
- The chain works per pixel only. Neighbourhood effects (blurs, convolution) are not chained.
- Colour adjustment and photo effects rows stay partial: `colorfx.rs` has the maths but nothing is wired to it.
- Not verified on the VM or Midget. Not built.
