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
- `springboard/render/src/rendering_intent.rs`: `Intent` (an alias of `cg::color::RenderingIntent`, extended with `ALL`,
  `name` and `adapts_white`), `IntentTransform` (a conversion under one intent, with optional `BlackPoints`) and
  `compensate_black`, the affine black point map. Perceptual compresses out-of-gamut colours toward the neutral grey
  of their mean; saturation clips them; absolute uses `colorspace::matrix_absolute`, which skips white adaptation.
  Nothing outside its tests calls it yet. Built on parity/team-a-09; not yet verified on Midget.
- Both modules are registered in `springboard/render/src/lib.rs`, one line each. Both have unit tests covering
  the matrix math, transfer curves, white-point mapping, round trips, gamut clamping, buffer sizes and byte order.
- `springboard/render/src/icc.rs` (extended): `Profile::matrix_shaper` reads the colorants (`rXYZ`, `gXYZ`, `bXYZ`) and
  tone curves (`rTRC`, `gTRC`, `bTRC`: `curv` identity, gamma and sampled forms, `para` function types 0 to 4) of an RGB
  matrix-shaper profile into `MatrixShaper`. `MatrixShaper::convert` takes encoded profile colours into any named space
  through the D50 connection space. `Profile::vcgt` reads a display's video card gamma table (`vcgt`, table and formula
  forms) and `Vcgt::ramp` samples it as 16-bit ramps. The header, tag table and description parsing from earlier rounds
  are unchanged; the description reader now shares the tag-body helper.
- `springboard/render/src/colorspace.rs` (extended): `from_xyz_d50(space)` maps the ICC D50 connection space into a named
  space, adapting the white point with Bradford. `apply` is public for the same callers.
- `springboard/sdk/src/colorsync.rs`: the app-facing module `bifrost_app::colorsync`. `ProfileConverter::open` takes the
  bytes of an RGB matrix-shaper profile and a destination `Space`; `apply_u8` and `apply` convert colours. `gamma_ramps`
  returns a display profile's gamma table as three 16-bit ramps. Errors are `ColorError` (not a profile, not a matrix
  shaper, no gamma table). Registered in `springboard/sdk/src/lib.rs` by one line.
- The HDR models (`hdr_transfer.rs`, `hdr_metadata.rs`) already in the tree are listed in the matrix row for HDR; this
  round did not change them.
- Built on parity/team-j-colorsync; not yet verified on Midget. The unit tests were written but not run (the lead builds).


Pure std: no crates, no `unsafe`, no hardware, no services.

## Remaining gaps

- No lookup-table ICC transforms (A2B, B2A, mft and mAB tags) and no profile-to-profile pipeline; `ColorSyncTransformCreate`
  and `ColorSyncTransformConvert` have only the matrix path.
- No profile creation, tag writing or embedding (`ColorSyncMutableProfile`, `ColorSyncProfileCopyData`).
- Decoded JPEG and PNG stay sRGB (`springboard/render/src/imgdec.rs`), and PDF `ICCBased` images are still drawn by
  component count (`springboard/render/src/cg/pdf/image.rs`); the working spaces are not applied to images yet.
- No display path to the hardware: `springboard/src/bare/kms.rs` reads `gamma_size` only, and `Vcgt` is not loaded into
  the CRTC. Night Shift does not read the display profile.
- Rendering intents and black point compensation exist as models (`rendering_intent.rs`), but an ICC profile's own
  intent tables are not read, and no image path applies them yet.
- No HAGC (SMPTE ST 2094-50) metadata; the PQ and HLG curves (`hdr_transfer.rs`) and EDR headroom are not wired into video
  output, and `nv12.rs` selects Rec.601 or Rec.709 by frame height only.
- Colour management modules (CMM plug-ins) remain `n/a`.
- Tests were written but not run in this round (the build and test stage runs them).
