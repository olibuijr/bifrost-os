# Parity: sdk-graphics-media (round 75)

Slice `sdk-graphics-media`. Matrix rows: `docs/parity-matrix/os/` entries "Core Image" (Graphics and Games) and
"Core Audio Types" (Media). The matrix is not edited here; the lead closes rows. Rows were read with
`tools/bifrost.py pc parity gaps` and `show`; the matrix text is often stale (the SDK already had image filters,
an audio client and a microphone gate), so each change below was checked against the code, not the row.

## Evidence

- No macOS 27 reference file was opened for this slice. The reference extraction under `~/Projects/macos-ref/extract/`
  was not used, and no REA evidence id was taken. The API names come from the matrix rows. Treat the behaviour below
  as the BifrOSt design, to check against the reference when the lead has the files.

## Core Image (Graphics and Games)

- API: `bifrost_app::image_filter::FilterChain` (`sdk/src/image_filter.rs`) runs named filters from
  `bifrost_render::effect_chain`. Two filters were missing from the catalogue and are added:
  - `gamma` (power curve, input `power`, range 0.1 to 3, default 1): the colour function is `colorfx::gamma`.
    It plays the role of the gamma adjustment filter (CIGammaAdjust in the matrix row), in the colour adjustment group.
  - `threshold` (input `threshold`, range 0 to 1, default 0.5): the pixel becomes black or white by Rec. 601 luma.
    It plays the role of the colour threshold filter, in the colour effect group.
- BifrOSt path: `springboard/render/src/colorfx.rs` (`gamma`, `threshold`), `springboard/render/src/effect_chain.rs`
  (catalogue, ranges, categories, apply), unit tests in both files.
- Done: an app reaches both through `FilterChain::push(Filter::named("gamma"))`; the filter browser lists them.
- Still open: the row's other items (lazy image graph, render destinations, kernels, feature detection, distortion,
  stylize, halftone and tile families). Needs Midget check only for visual output (no screen check was done).

## Core Audio Types (Media)

- Shared format: `bifrost_audio_types::StreamFormat` (audio/types crate, no dependencies) is now the one format
  description used by the recorder path. `springboard/src/media/capture.rs`:
  - `pcm16(rate, channels)` builds the take format (16-bit signed interleaved PCM, a 2-byte frame per channel).
  - `wav_bytes(&StreamFormat, &[i16])` writes the canonical 44-byte header from the format: channels, sample rate,
    byte rate, block align and bit depth. It replaces `wav_bytes(rate, channels, samples)`.
  - `Take { format, samples }` replaces `Take { rate, channels, samples }`; `Recorder` keeps `pub format` in place of
    the `pub rate` and `pub channels` fields.
- App-facing: `bifrost_app::audio_format` (`sdk/src/audio_format.rs`) re-exports `StreamFormat`, `FormatId` and
  `FormatFlags`, `pcm16`, and `duration_ms(format, bytes)`, which counts whole frames and returns `None` for planar
  or unrated formats.
- Migrations (adopt and delete):
  - `springboard/audiod/src/main.rs`: `RecordStop` writes the WAV from `take.format` and computes the take length
    from `take.format.sample_rate` and `take.format.channels`.
  - `springboard/apps/voicememos-desktop/src/store.rs` and `voicememos.rs`: every `wav_bytes(8000, 1, ...)` and the
    trim path call `wav_bytes(&pcm16(...), ...)`; the test call sites migrate the same way.
  - Voice Memos bumped to 1.6.4 (`manifest.toml`, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`).
- Not changed: `pcm16_format` (the voice memo header reader in `store.rs`) still returns a loose `(rate, channels)`
  pair; a later change can return a `StreamFormat` from it. `audio/types` and the audio engine already have their own
  format types (`ui/src/audio_engine.rs` `Format`, `ui/src/audio_render.rs` `StreamFormat`); folding them into the
  shared one is a separate migration and was not started here.
- Platform: the recorder module is `cfg(target_os = "linux")`, so the SDK re-export is Linux only, like the rest of
  the capture path.
- Needs Midget check: record a take in Voice Memos, trim it and check the saved file length and header with
  `xxd`; confirm `audiod` still writes the 16-bit mono take.

## Build status

Nothing was compiled or tested in this round (the brief forbids builds). The changes were checked by reading the
call sites. The lead should build `bifrost-render`, `bifrost-audio-types`, `bifrost-app` (sdk), `springboard`
(capture), `bifrost-audiod` and `bifrost-voicememos`, and run the `colorfx`, `effect_chain`, `capture` and
`audio_format` unit tests.
