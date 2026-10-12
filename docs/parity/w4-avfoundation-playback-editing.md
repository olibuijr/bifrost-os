# Parity wave 4: AVFoundation playback and editing

Branch `parity/w4-avfoundation-playback-editing`, built on `parity/integration` (waves 1 to 3). Pure Rust, std only,
no builds run in this wave. Modules live in `springboard/ui/src/` (`bifrost_ui`), next to the wave 1 composition
and audio mix models they extend.

## What was built

- **Video output settings (`springboard/ui/src/video_settings.rs`).** A `VideoSettings` model for one video output:
  codec (`Codec::H264`, `Codec::Hevc`), frame width and height (non-zero and even, because both codecs code 4:2:0
  chroma), an optional average bit rate in bits per second, and an optional maximum key frame interval in frames.
  `estimated_bytes(seconds)` gives the output size at the average bit rate, rounded up. `content_rect(w, h, fit)`
  places a source in the frame for `Fit::Stretch`, `Fit::AspectFit` (letterboxed, centred) or `Fit::AspectFill`
  (scaled to cover, overflow cropped by the caller). No frames are encoded.
- **Layer opacity ramps and cross dissolve (`springboard/ui/src/video_transition.rs`).** `LayerOpacity` keeps the
  opacity changes of one track as ramps over `composition::TimeRange` ticks: `set_ramp(from, to, range)` moves
  linearly across the range and holds `to` after it; `set_opacity(value, at)` holds a level from a tick. A later
  ramp that starts on the same tick replaces the earlier one, and a later start ends an earlier ramp there.
  `VideoMix` holds one `LayerOpacity` per track and offers `cross_dissolve(outgoing, incoming, range)`, which fades
  the outgoing track from 1 to 0 and the incoming track from 0 to 1 over the range. A track without a started ramp
  is fully opaque. Values are clamped to 0 to 1; NaN and infinite values, and empty or negative ranges, are refused.

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`. Each has a
`#[cfg(test)] mod tests` block with hand-worked expectations (six tests in `video_settings`, six in
`video_transition`).

## Official API names mirrored

Apple facts are from the official pages `https://developer.apple.com/documentation/avfoundation/video-settings`
(encoder keys: codec, width, height, average bit rate, maximum key frame interval, pixel aspect ratio) and
`https://developer.apple.com/documentation/avfoundation/avvideocompositionlayerinstruction` (`setOpacityRamp(fromStartOpacity:toEndOpacity:timeRange:)`,
`setOpacity(_:at:)`, `opacityRamp(at:)`, and the layer instruction for a source track). The Swift-facing names are
the reference; the BifrOSt names are Rust names over the same rules. The types are not bridged to any AVFoundation
type, and no key strings are exported.

Not mirrored: clean aperture, colour properties and transfer functions, entropy mode, profile level, frame rate
keys, the HDR and log transfer functions, transforms and crop ramps, the custom compositor class, and the
validation delegate.

## REA evidence

- `ev_488149439bfc50c9e2ea5e315f7cccaae768ab4b0cdda392d7ec4fe47f6d65f1` (Video settings page, rendered headings and key list).
- `ev_a879ea22f67ae8bfaaff18de38b862d3f6d6fc247612eaf3fc1cef9c175609a9` (AVVideoCompositionLayerInstruction page, opacity ramp methods).

## Matrix rows closed

The rows below are the ones this wave moves from `gap` to `partial`. Feature names are exactly as written in
`docs/parity-matrix/data/avfoundation-playback-editing.json`. The lead updates the matrix centrally.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| avfoundation-playback-editing | Video settings | partial | springboard/ui/src/video_settings.rs |
| avfoundation-playback-editing | Video effects and compositing | partial | springboard/ui/src/video_transition.rs |

Still open in this framework, not touched here: Media reading and writing, Sample buffer playback, QuickTime movies,
and the `os` rows (Media playback, Video decoding, Audio output and routing), which need an app-facing player or
session over the shell.

## Limits

- Nothing is wired into the shell or into apps. There is no encoder, so `VideoSettings` describes an output
  that nothing writes yet.
- The dissolve is opacity only: no transforms, no custom compositor, and no per-frame callback.
- Ticks are `i64` and opacities are `f32`. Ramp values are exact for the dyadic examples in the tests; other values
  round as `f32` does.
- Untested on the VM or Midget: the models have host unit tests only, and these tests have not been run in this wave.
