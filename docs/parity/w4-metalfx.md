# Wave 4: MetalFX temporal scaling and frame interpolation (pure Rust models)

Branch `parity/w4-metalfx`, started from `parity/integration` (wave 3). The matrix data in
`docs/parity-matrix/` is not edited here; the lead imports the rows below.

## What was built

Two pure CPU models in `springboard/render/src/`, std only, no GPU pass, no unsafe, no hardware access.

- `temporal_upscale.rs` (`TemporalScaler`, `Accumulation`). Jittered low-resolution frames accumulate into a
  full-resolution history. The model has a Halton (2, 3) jitter sequence of eight phases, reprojection with a
  per-pixel motion field (input pixels), history rejection on frame edges and on depth change larger than 0.02, a
  3x3 neighbourhood clamp of the history, and a blend weight of 0.9 (standard) or 0.95 (denoised). The current
  colour is upscaled by the existing spatial filter (`upscale::upscale_f32`, linear mode).
- `frame_interp.rs` (`interpolate`, `FrameError`). The frame at time `t` between two frames. Without motion it is a
  cross-fade. With motion it is a backward warp of both frames, blended with weights `1 - t` and `t`. The endpoints
  return the input frames exactly.

Both files carry `#[cfg(test)]` tests with hand-worked values (Halton phases, the clamp and blend arithmetic, the
half-pixel shift `[0, 0.5, 0.5, 0]`, the refused inputs).

## Apple APIs mirrored (official documentation)

- `MTLFXTemporalScaler` (macOS 13.0). Official page: https://developer.apple.com/documentation/metalfx/mtlfxtemporalscaler
  Mirrored: temporal accumulation from colour, depth and motion inputs, and the output size from the input size.
- `MTLFXTemporalDenoisedScaler` and its descriptor (macOS 26). Mirrored only as the `Accumulation::Denoised` mode.
  No denoising filter exists in the model.
- `MTLFXFrameInterpolator` (macOS 26.0). Official page: https://developer.apple.com/documentation/metalfx/mtlfxframeinterpolator
  Mirrored: frame generation from two frames and a time value.
- `MTLFXTemporalScaler` landing and collection: https://developer.apple.com/documentation/metalfx

Not mirrored: the descriptors and their factory methods (`makeTemporalScaler(device:)`), the `encode(commandBuffer:)`
command encoding, the GPU texture types, and the MTL4 protocol variants. These are tied to the Metal runtime, which
BifrOSt does not have.

## REA evidence

- `ev_6393af496865b0ecb9584add1491c03160d66be065d8c699476b12ddba8a3979`: MetalFX collection page (the landing
  text: temporal scaler needs colour, depth and motion; spatial scaler needs colour only).
- `ev_e95a321d0734cd1430609e58a888fa79319852503e9dec0bea8eb44b388f9a65`: MTLFXTemporalScaler page (inputs,
  `encode(commandBuffer:)`, macOS 13.0+).
- `ev_483597e4d472f0fde8d471960a7073d69b55d3a48e1522386e487775803ac401`: MTLFXFrameInterpolator page
  (`encode(commandBuffer:)`, macOS 26.0+).

## Matrix rows closed (status `partial`)

| framework slug | feature (as in data/metalfx.json) | new status | BifrOSt path |
|---|---|---|---|
| metalfx | Temporal upscaling (MTLFXTemporalScaler) | partial | springboard/render/src/temporal_upscale.rs |
| metalfx | Temporal denoised upscaling (MTLFXTemporalDenoisedScaler) | partial | springboard/render/src/temporal_upscale.rs (`Accumulation::Denoised`) |
| metalfx | Frame interpolation (MTLFXFrameInterpolator) | partial | springboard/render/src/frame_interp.rs |

Still open in MetalFX after this wave: the GPU path (no scaler runs on the GLES presenter in
`springboard/src/gpu/wl.rs`), the app-facing SDK API (none in `springboard/sdk/src/`), the colour-space argument
of the temporal path, the reactive mask, and the denoiser filter itself. The `os` row for command-buffer encoding
stays `os`.

## Verification

Not built or run in this wave, per the wave rules. The name guard `python3 tools/check-names.py` passes with no
hits.
