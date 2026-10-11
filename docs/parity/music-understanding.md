# Music Understanding parity

Official reference: https://developer.apple.com/documentation/musicunderstanding (macOS 27.0; Mac Catalyst 27.0).
Matrix row: `docs/parity-matrix/data/music-understanding.json` (status `partial`, was `gap`).

## What was built

Two gaps were addressed in pure Rust with std only. No new crates, no `unsafe`, no hardware or network.

1. **Analysis session and result types.** `springboard/ui/src/music.rs` (crate `bifrost-ui`, module `music`):
   - `MusicUnderstandingSession` mirrors `init(audioProvider:)` for a whole buffer (`new`, mono samples) and a
     channel mix-down (`from_interleaved`). It has `analyze()`, `analyze(for:)` (`analyze_for`) and `cancel()`.
   - `AnalysisType` (Key, InstrumentActivity, Loudness, Pace, Rhythm, Structure). `AnalysisType::SUPPORTED` lists
     the analyses that run: only Rhythm.
   - `SessionResult` carries only the `rhythm` field. Other analyses are not in it.
   - `TimedValue<T>` and `RangedValue<T>` mirror the support structures.
   - `MusicUnderstandingError`: `EmptyAnalysisSet` (`emptyAnalysisSet`), `InvalidAsset` (`invalidAsset`),
     `Unsupported(AnalysisType)`, `Cancelled`. `internalError` and `hasProtectedContent` have no counterpart:
     there is no decoder here and no DRM.
   - `springboard/sdk/src/music_understanding.rs` re-exports the types for apps as `bifrost_app::music_understanding`
     and adds `analyze_interleaved` (one call for interleaved PCM).

2. **Rhythm analysis.** `analyze_rhythm` mirrors `RhythmResult` (`beats`, `bars`, `beats_per_minute`):
   - an onset-energy envelope from 10 ms frames (log-compressed energy, positive changes only);
   - the tempo from the envelope's autocorrelation over 60 to 200 BPM, weighted by a prior centred on 120 BPM
     (a log-normal with a one-octave width) so a double or half tempo loses ties;
   - a beat grid whose phase, searched in half-frame steps, collects the most onset energy;
   - bars are every fourth beat from the first beat.
   Silence gives no beats and 0 BPM.

## Choices and limits

- Times are seconds from the start of the buffer. Apple's `beats` is a list of timestamps; this is the same shape.
- `cancel()` is checked between analyses and at the end, so a long single analysis cannot be stopped mid-run.
  It resets the flag when `analyze_for` starts, so a cancel issued before the call does nothing.
- Rhythm is one tempo for the whole buffer. It is not tested on real music; the unit tests use click tracks at
  120 BPM (44.1 kHz and 22.05 kHz), a grid check, a bar check and silence.

## Official API mirrored

- `MusicUnderstandingSession`: `init(audioProvider:)` (partly), `analyze()`, `analyze(for:)`, `cancel()`.
- `RhythmResult`: `beats`, `bars`, `beatsPerMinute`.
- `MusicUnderstandingSession.SessionResult` (the `rhythm` field only), `TimedValue`, `RangedValue`, `AnalysisType`,
  `MusicUnderstandingError` (four of six cases).

## REA evidence

- `ev_b43f3bf67b5f530fe587b02d014b2ed883f1d278dadef09ee1794d9a27614a5f`: rendered landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/musicunderstanding`), this round.
- `ev_5ca98fba11b7d0d8290a0ab5f2bb01719eeda945e38cfc42f1cb97e6eccf7777`: the earlier landing-page evidence in the matrix.
- Official docc topics read: `musicunderstanding`, `musicunderstandingsession`, `loudnessresult`, `rhythmresult`,
  `musicunderstandingerror`, `analysistype`, `musicunderstandingsession/sessionresult`,
  `musicunderstandingsession/timedvalue`.

## Remaining gaps

- Key (`KeyResult`), pace (`PaceResult`), structure (`StructureResult`), instrument activity
  (`InstrumentActivityResult`, with source separation). Each still returns `Unsupported`.
- Loudness (`LoudnessResult`: momentary and short-term values, and the `loudnessResults` sequence). The ITU-R BS.1770
  code in the DAW lab (`springboard/apps/daw/src/lab.rs`) is app-private and should move into a shared module before
  the session can use it.
- Input: no file-asset session, no streaming buffer session. Decoding is the app's job today.
- Rhythm: tempo changes, meter, and validation on real recordings.
- Async results and partial delivery.
