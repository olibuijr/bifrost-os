# Wave 3: MetalFX spatial upscaling (parity/w3-metalfx)

## What was built

`springboard/render/src/upscale.rs` is a pure CPU spatial upscaler. It is a separable Catmull-Rom (bicubic,
a = -0.5) filter over `[f32; 3]` samples (`upscale_f32`) and over 8-bit RGB bytes (`upscale_rgb8`). Taps outside the
input clamp to the edge. The horizontal pass runs first, then the vertical pass. The weights sum to one, so flat areas
stay flat.

`ColorProcessing` says how the samples are encoded, and so how the filter treats them:

- `Linear`: samples are linear light. Filtered as given and clamped to 0..=1.
- `Perceptual`: samples are sRGB-encoded. Decoded to linear light with `colorspace::Space::Srgb`, filtered, clamped and
  encoded again.
- `Hdr`: linear light where values above 1 are kept. Filtered as given and not clamped. Only `upscale_f32` accepts it,
  because 8-bit storage cannot hold the overshoot.

`UpscaleError` reports zero sizes, a sample count that does not match the source size, and HDR bytes.

The module reuses the sRGB transfer curve already in `springboard/render/src/colorspace.rs`; it adds no new transfer
function.

Not built: no GPU pass (the presenter in `springboard/src/gpu/wl.rs` is untouched), no edge-directed analysis, no
temporal or motion input, no SDK call for apps. The module is not wired into the shell or the image path
(`Image::resized` in `springboard/render/src/gfx.rs:360` is unchanged). It has host unit tests only; it has not been
built or run on the VM or Midget in this wave.

## Official API names mirrored

- `MTLFXSpatialScaler`, `MTLFXSpatialScalerDescriptor`: an upscaling effect that "spatially analyzes an input
  texture" to make a higher-resolution texture
  (https://developer.apple.com/documentation/metalfx/mtlfxspatialscaler, DocC via `tools/parity/docc.py`).
  The Catmull-Rom filter mirrors only the spatial part, not the analysis.
- `MTLFXSpatialScalerColorProcessingMode` with `linear`, `perceptual` and `hdr`
  (https://developer.apple.com/documentation/metalfx/mtlfxspatialscalercolorprocessingmode). The mapping is
  `ColorProcessing::Linear`, `Perceptual` and `Hdr`. Apple's text says the mode describes the input and output
  textures; the semantics here follow that wording.

## REA evidence

- `ev_1892b1e770e0125c51d4a9f5a858daa16155afe2c55a3f9ad2c13c048803cf2c`: rendered official MetalFX landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/metalfx`). The page lists the spatial scaler
  and its colour processing enum.
- `ev_0ce9d57e5d0b4a3c283da154ba0b0a14527d6bfea12a70f43f67e89c4f6c7348`: the evidence already recorded in
  `docs/parity-matrix/data/metalfx.json`, used for the rows below.

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| MetalFX | Spatial upscaling (MTLFXSpatialScaler) | partial | springboard/render/src/upscale.rs |
| MetalFX | Spatial colour processing modes | partial | springboard/render/src/upscale.rs |

Both rows stay partial: there is no GPU path, no edge-aware analysis and no app-facing API. The temporal scaler,
the temporal denoiser, frame interpolation and the Metal command-buffer encoding rows are unchanged.

## Tests

`cargo test` is not run in this wave. Each test's expected values are worked out by hand in the test comments:
Catmull-Rom weights at t = 0 and 0.5, the flat-raster invariant, the HDR overshoot -0.0703125 and 1.0703125 on a
2-to-4 edge, the clamped Linear result, the 128 (linear) versus 188 (perceptual) midpoint, and the error cases.
