# ColorSync parity (colour transforms, pixel layout, working spaces)

Official reference: https://developer.apple.com/documentation/colorsync (collection with Color transforms,
Pixel format and data layout, Color profiles, Headroom Adaptive Gain Curve, Color devices, Color management
modules, Supporting types and conventions).

## What was built

- `springboard/render/src/colorspace.rs`: named RGB working spaces `Space` (sRGB, Display P3, Adobe RGB (1998),
  ROMM RGB). Each has its CIE xy primaries, white point (D65, or D50 for ROMM) and transfer curve. `matrix(from, to)`
  builds the 3x3 linear-light conversion through XYZ, with Bradford chromatic adaptation when the whites differ.
  `Transform` caches that matrix and converts colours (`apply` for encoded floats, `apply_u8` with clamping).
  Negative and above-1 channels are mirrored through the transfer curve, so out-of-gamut results stay defined.
- `springboard/render/src/pixel_layout.rs`: `Layout { depth, alpha }` with `Depth` (8, 16, float32) and `Alpha`
  (none, straight, premultiplied). It gives channel counts, row and buffer sizes and a length check. `to_bytes`
  serialises the renderer's premultiplied `u32` pixels into any of those layouts. `from_bytes_u8` reads 8-bit
  buffers back. `unpremultiply`, `widen` and `narrow` are the helpers.
- `springboard/render/src/rendering_intent.rs`: `Intent` (perceptual, relative colorimetric, saturation, absolute
  colorimetric), `IntentTransform` (a conversion under one intent, with optional `BlackPoints`) and
  `compensate_black`, the affine black point map. Perceptual compresses out-of-gamut colours toward the neutral grey
  of their mean; saturation clips them; absolute uses `colorspace::matrix_absolute`, which skips white adaptation.
  Built on parity/team-a-09; not yet verified on Midget.
- Both modules are registered in `springboard/render/src/lib.rs`, one line each. Both have unit tests covering
  the matrix math, transfer curves, white-point mapping, round trips, gamut clamping, buffer sizes and byte order.

Pure std: no crates, no `unsafe`, no hardware, no services.

## Official API names mirrored

| BifrOSt | Apple reference |
|---|---|
| `colorspace::Transform` | ColorSyncTransform (colour transforms, `ColorSyncTransformConvert` by matrix only) |
| `colorspace::Space::DisplayP3`, `AdobeRgb`, `RommRgb`, `Srgb` | kColorSyncDisplayP3Profile, kColorSyncAdobeRGB1998Profile, kColorSyncROMMRGBProfile, kColorSyncSRGBProfile |
| `pixel_layout::Layout`, `Depth`, `Alpha` | ColorSyncDataLayout, ColorSyncDataDepth, ColorSyncAlphaInfo |
| `rendering_intent::Intent`, `BlackPoints` | Rendering intents, black point compensation |

The names are only the concepts. The Apple symbols are not used in code (owner rule).

## REA evidence

- `ev_bf6c8801f818b6f862c7df1df543d68614564c66b383f0c8bf3ca9b3b53923c8`: rendered landing page of the ColorSync
  framework (topics, Color conversion, Profile and HDR metadata, System color management).
- DocC structure: `python3 -I tools/parity/docc.py colorsync` (same topic list).

## Remaining gaps

- No ICC profile parser and no profile-table transform. `ColorSyncTransformCreate` and `ColorSyncTransformConvert`
  have no equivalent beyond the published-primaries matrix.
- PDF `ICCBased` images are still drawn by component count (`springboard/src/shell/books/engine/pdf/image.rs`);
  the working spaces are not applied to them.
- Decoded JPEG and PNG stay sRGB (`springboard/render/src/imgdec.rs`); wide-gamut input is not tagged.
- No display profile or gamma ramp: `springboard/src/bare/kms.rs` reads `gamma_size` only.
- Rendering intents and black point compensation exist as models (`rendering_intent.rs`), but an ICC profile's
  own intent tables are not read, and no image path applies them yet.
- No HDR pipeline (PQ, HLG, extended range, Headroom Adaptive Gain Curve metadata).
- Colour management modules (CMM plug-ins) remain `n/a`.
- No app-facing SDK wrapper: `springboard/sdk/src` does not expose `colorspace` or `pixel_layout` yet.
- Tests were written but not run in this round (the build and test stage runs them).
