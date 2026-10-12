# Music Understanding parity, wave 5: key over time and structure

Official reference: https://developer.apple.com/documentation/musicunderstanding (macOS 27.0; Mac Catalyst 27.0).
Matrix data: `docs/parity-matrix/data/music-understanding.json`. This wave closes two `gap` rows, both now `partial`.

## What was built

Pure Rust, std only. No new crates, no `unsafe`, no hardware or network. Code is in the existing music model,
`springboard/ui/src/music.rs` (module `music`, crate `bifrost-ui`), with the app-facing exports in
`springboard/sdk/src/music_understanding.rs`.

1. **Key over time (`AnalysisType.key`, `KeyResult`, `KeyResult.Mode`).**
   - `chroma_frames` measures non-overlapping, Hann-windowed frames at the equal-tempered tones MIDI 48 to 95 with the
     Goertzel algorithm and folds the power into 12 pitch classes.
   - `key_from_chroma` correlates a 12-bin chroma vector (Pearson) with the Krumhansl and Kessler (1982) major and minor
     profiles at each of the 12 tonics. A flat or silent chroma has no key.
   - `analyze_key` sums chroma over windows of `KEY_WINDOW_SECONDS` (10 s) and gives one `RangedValue<Key>` per window.
     Silent windows give no range.
   - `Key { tonic: u8, mode: Mode }`, with C = 0 through B = 11.

2. **Structure (`AnalysisType.structure`, `StructureResult`).**
   - `segment_frames` takes per-frame feature vectors and computes a novelty curve: the Euclidean distance between the mean
     feature of the `window` frames before a point and the mean of the `window` frames after it. A boundary is a local
     peak at or above `min_novelty`; peaks closer than `window` frames to an earlier boundary are dropped.
   - `structure_from_chroma` scales each chroma frame to sum 1, runs `segment_frames` with
     `STRUCTURE_WINDOW_FRAMES` (16 frames, about 1.5 s) and `STRUCTURE_MIN_NOVELTY` (0.3), and returns
     `StructureResult { segments: Vec<Range<f64>> }` in seconds.

3. **Session wiring.** `AnalysisType::SUPPORTED` is now `[Rhythm, Key, Structure]`. `SessionResult` gained `key` and
   `structure` fields. `analyze_for` computes the chroma frames once, and only when Key or Structure is requested.
   Loudness and instrument activity still fail with `Unsupported`.

The constants `STRUCTURE_WINDOW_FRAMES` and `STRUCTURE_MIN_NOVELTY` are chosen by hand, not tuned on recordings.

## Official API names mirrored

| Official name | BifrOSt name | Notes |
|---|---|---|
| `KeyResult` | `KeyResult` | official `ranges` is `[RangedValue<KeyResult.KeySignature>]`; here `Vec<RangedValue<Key>>` |
| `KeyResult.Mode` (`major`, `minor`) | `Mode::Major`, `Mode::Minor` | same two cases |
| `KeyResult.Tonic` (17 enharmonic cases) | `Key::tonic: u8` (12 pitch classes) | see gaps |
| `KeyResult.KeySignature` | none | see gaps |
| `StructureResult.segments` | `StructureResult::segments` | one level only |
| `StructureResult.sections`, `.phrases` | none | see gaps |
| `AnalysisType.key`, `AnalysisType.structure` | `AnalysisType::Key`, `AnalysisType::Structure` | |

## REA evidence

- Landing page: `ev_cdfefe20de4b1a456cf2abb532345eddaecc867c95e6bc365f38d109cdc4475a`
- `KeyResult`: `ev_6a0c70e6f6f13ca4c7bb27e79d8a8246b38187336a7a4ade1d688f5444019f42`. The rendered page says the
  tonic has 17 enharmonic spellings kept as distinct cases.
- `StructureResult`: `ev_d63d0e48382a85dbaef5ed565a1acea24b65999ba88a7363cd1ace17a1685cae`. The rendered page says a
  section has one or more segments and a segment has one or more phrases.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Music Understanding | Key (musical key over time) | partial | `springboard/ui/src/music.rs` (`analyze_key`, `key_from_chroma`, `chroma_frames`); `springboard/sdk/src/music_understanding.rs` |
| Music Understanding | Structure (sections, segments, phrases) | partial | `springboard/ui/src/music.rs` (`segment_frames`, `structure_from_chroma`); `springboard/sdk/src/music_understanding.rs` |

## Tests

Host unit tests in `springboard/ui/src/music.rs`, module `key_structure_tests`, with expected values worked out by hand:

- The major profile at tonic 0 gives C major. A major profile rotated by 7 semitones gives G major (tonic 7). A minor
  profile rotated by 9 gives A minor (tonic 9, minor).
- Flat chroma and silence give no key.
- Two windows of four frames (C major, then G major, 0.5 s per frame) give ranges `0.0..2.0` and `2.0..4.0`.
- A 440 Hz sine gives pitch class 9 (A) in each 4096-sample frame; silence gives zero chroma.
- Ten frames of one texture then ten of another, window 3, threshold 0.5: the only boundary is frame 10, giving
  segments `0..10` and `10..20`. Frames 9 and 11 score sqrt(8)/3, below the peak at sqrt(2).
- A threshold of 1.5 gives one segment; an input shorter than twice the window gives one segment; empty input gives none.

The session tests in `springboard/ui/src/music.rs` and `springboard/sdk/src/music_understanding.rs` were updated: Key
and Structure are no longer "unsupported" examples; Loudness and InstrumentActivity are used instead.

Not run: no build and no test run in this wave, per the task rules. The tests and the build are for the integration
round.

## Gaps that remain

- Key: 12 pitch classes instead of the 17 enharmonic `Tonic` cases; no key signature (`KeySignature`); chroma uses
  4096-sample frames, so low tones blur into their neighbours; the key profiles are not tuned on recordings.
- Structure: only `segments` exists. `sections` (named parts such as intro or chorus) and `phrases` are not produced.
  Boundaries come from chroma novelty only, with hand-chosen constants.
- Session input is still a whole buffer; there is no streaming result, and no buffer-stream input.
- Not wired into any app or the shell, and not run on the VM or Midget yet.
