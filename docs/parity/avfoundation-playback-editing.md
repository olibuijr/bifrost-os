# AVFoundation (Playback, Editing, Common): parity pass

Official documentation: https://developer.apple.com/documentation/avfoundation (DocC topics read with
`tools/parity/docc.py`; the rendered landing page recorded with REA evidence `ev_2039682c451f2f60bfa0bb335a4a618725a08d5d1d420efa070e2c5faf9d366f`).
The earlier framework-level evidence in the matrix is `ev_0a8eac4fffbfe8853cb7941e11e3a35275f22d910252c983eb422fcb5cc92d68`.

## What was built

Two pure models in `bifrost-ui`, std only, no media decoding:

- `springboard/ui/src/composition.rs`: a composed timeline. A `CompositionTrack` is a list of segments that tile the
  timeline from 0; each segment is either media from a source track (with its source start and duration) or an empty
  gap. Operations: `insert` (splices a source range in and moves later media back), `insert_empty`, `remove` (closes
  the gap), `scale` (changes the composed length, keeps the source duration), `segment_at`. `Composition` applies
  insert-empty and remove to every track as one edit, and refuses the whole edit when any track cannot take it.
- `springboard/ui/src/audio_mix.rs`: per-track volume over time. `TrackVolume` holds levels from a time on and linear
  ramps over a span; a later change at its own start ends an earlier ramp; an empty ramp is a jump; volumes are clamped
  to 0 to 1 and non-finite values are refused. `AudioMix` keeps one entry per track.

Both modules are registered in `springboard/ui/src/lib.rs` (one `pub mod` line each). Tests sit in each module:
six in `composition.rs` and five in `audio_mix.rs`.

## Official API names mirrored

- Composition: `AVMutableComposition` (`addMutableTrack`, `removeTimeRange`, `scaleTimeRange`, `insertEmptyTimeRange`,
  `insertTimeRange(_:of:at:)`), `AVMutableCompositionTrack` (`insertTimeRange`, `insertEmptyTimeRange`,
  `removeTimeRange`, `scaleTimeRange`, `segments`).
- Audio mix: `AVAudioMix` (`inputParameters`), `AVMutableAudioMixInputParameters` (`init(track:)`, `setVolume(_:at:)`,
  `setVolumeRamp(fromStartVolume:toEndVolume:timeRange:)`, `trackID`).

Names are not used in the code (owner rule). `tools/check-names.py` passes.

## Deliberate differences

- Time is integer ticks of the track's time scale in the composition model, and seconds (`f64`) in the audio model.
  Apple uses `CMTime` values; no `CMTime` type exists here.
- Source references are a track id plus a range. There is no asset object, so the composition cannot load a file.
- Segments are split in proportion when a source is cut inside a scaled segment, so source positions can be off by a
  tick after repeated scaling. Scaled boundaries are rounded to the nearest tick, and zero-length pieces are dropped.

## Not built, and why

- No app-facing SDK API: the models are in `bifrost-ui` only. Exposing them in `springboard/sdk/src` needs an
  asset/track loader first, which needs demux access that is still shell-internal.
- Nothing applies these models: the player, `audiod` and the mixer do not read a composition or a volume ramp.
- Video composition (transitions, custom compositor, layer animation), export and writing, captions, HLS, FairPlay,
  AirPlay, sample buffer renderers and the audio tap processor remain gaps.

## Matrix changes

`docs/parity-matrix/data/avfoundation-playback-editing.json`: "Composite assets" gap to partial; "Audio mixing" os to
partial; top-level status stays partial; summary, gaps and the top-level REA evidence updated.
