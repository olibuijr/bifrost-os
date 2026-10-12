# Wave 5: Video Toolbox parity (HDR metadata and RAW demosaic)

Two gap rows of the Video Toolbox framework are now partial models in pure Rust (std only, no
hardware access, no unsafe). Both are pure models: they run on the host, are not yet wired into the
shell or any app, and have not been built or run on the VM or Midget in this wave.

## What was built

### HDR per-frame light metadata: `springboard/render/src/hdr_metadata.rs`

- PQ electro-optical transfer (`pq_to_nits`) and its inverse (`nits_to_pq`), with the SMPTE ST 2084
  constants, peak 10000 cd/m2.
- HLG opto-electrical transfer (`hlg_oetf`) and inverse (`hlg_eotf`), with the ITU-R BT.2100 constants
  and a nominal peak of 1000 cd/m2 (`HLG_NOMINAL_NITS`). The HLG system OOTF is not applied.
- `code_to_nits` maps a 10-bit code under the `Transfer` tag from `springboard/render/src/buffer_attach.rs`
  (PQ or HLG only; other transfers are refused).
- `FrameLightSession` mirrors the shape of the official per-frame HDR metadata session: it is created
  with a frame rate and a format, and `attach` is called per frame with a `scene_change` flag. Each call
  returns `FrameLight` (frame index, frame peak, frame mean light, scene peak in cd/m2).

Apple parity gaps, stated explicitly:
- The Dolby Vision output format is not implemented. The format list holds only PQ and HLG.
- Metadata is returned to the caller. It is not attached to a pixel buffer, and no bitstream carries it.
- The frame average is a plain mean of the samples, not weighted by area or by display gamma.
- Only 10-bit luma codes are accepted. Chroma is not read.

### RAW Bayer demosaic and white balance: `springboard/render/src/bayer.rs`

- `CfaPattern` (RGGB, BGGR, GRBG, GBRG) with `channel_at(x, y)`.
- `demosaic(mosaic, width, height, pattern, gains)` returns interleaved 16-bit RGB. A photosite keeps its
  own sample. Each missing colour is the mean of that colour in the 3x3 neighbourhood, cut at the frame
  edge. Per-channel white balance gains apply after interpolation. Results are rounded and clamped to 65535.
- Errors: `EmptyFrame`, `SizeMismatch`, `InvalidGain(channel)`.

Apple parity gaps, stated explicitly:
- No RAW container decode and no camera-native input from the sensor. The input is a plain mosaic slice.
- No processing parameters. The official processing session exposes a parameter list and updates by key.
  The existing `springboard/render/src/raw_params.rs` model holds those parameters and is not connected to
  this demosaic, so the parameter list is not wired to `gains` yet.
- No colour matrix, tone curve, or Metal device. Demosaic is bilinear-style and runs on the CPU.

## Official API names mirrored

Official pages read through DocC (`tools/parity/docc.py`):
- VideoToolbox, collection: https://developer.apple.com/documentation/videotoolbox
- `VTHDRPerFrameMetadataGenerationSession` (`init(framesPerSecond:hdrFormats:)`,
  `attachMetadata(to:sceneChange:)`): https://developer.apple.com/documentation/videotoolbox/vthdrperframemetadatagenerationsession
- `VTHDRPerFrameMetadataGenerationSession.HDRFormat`: https://developer.apple.com/documentation/videotoolbox/vthdrperframemetadatagenerationsession/hdrformat
- `VTRAWProcessingSession` (`process(frame:)`, `updateParameter(values:)`, `processingParameters`):
  https://developer.apple.com/documentation/videotoolbox/vtrawprocessingsession
- `VTRAWProcessingSession.Parameter`: https://developer.apple.com/documentation/videotoolbox/vtrawprocessingsession/parameter

Mirror names in the Rust code: `FrameLightSession::new` and `attach` (metadata session), `demosaic` and
`CfaPattern` (RAW processing session).

## REA evidence

- `ev_84e972c47bb080a8cabbff313e063569dec059a70f3b3cab66a8f7a468debe10`: rendered page of
  `VTHDRPerFrameMetadataGenerationSession`, read with `tools/parity/rea-page.sh`.
- `ev_c1e8d57622d958a0efc4237de959926728bea8ba633aed12c24a6b7e0f9ccc5d`: rendered page of
  `VTRAWProcessingSession`, read with `tools/parity/rea-page.sh`.
- `ev_8d9d9d0fe1cf32381023d3cddfdb7ed9d489879706d7276fd201f21500922b18`: the framework landing page,
  already recorded in the matrix (not re-read in this wave).

## Matrix rows closed

These rows are for the lead to apply to `docs/parity-matrix/data/video-toolbox.json`. This wave does not edit it.

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| video-toolbox | HDR per-frame metadata | partial | springboard/render/src/hdr_metadata.rs |
| video-toolbox | RAW processing | partial | springboard/render/src/bayer.rs |

## Tests

Each module has a `#[cfg(test)] mod tests`. Expected values were worked out by hand from the rules the code
implements: PQ endpoints (0 and 1 map to 0 and 10000 cd/m2 exactly), PQ round trips, the HLG branch point at
1/12 (coded 0.5), frame peak and mean from known samples, scene-peak reset on a cut, input refusal, the 2x2
RGGB interpolation values, white balance and clamping, pattern tiles, and size and gain validation.

Not run: no build and no test run was done in this wave, per the task rules. The tests are unverified until a
build is run.

## Not built in this wave

- Compression session (encode path), multi-pass storage, and frame processing (super resolution, optical
  flow, motion blur): no pure-Rust encoder or ML path was attempted.
- App-facing decompression session (an "os" row): needs a crate boundary between the media stack and the SDK,
  which this wave did not change.
