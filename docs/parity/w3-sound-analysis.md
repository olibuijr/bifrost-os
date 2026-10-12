# Wave 3: Sound Analysis (partial)

Branch `parity/w3-sound-analysis`. Pure Rust, std only, in `springboard/ui` (`bifrost_ui`). Not built or run yet: the
wave rules forbid builds, so the tests were checked by hand and the integration build runs later.

## What was built

- `springboard/ui/src/sound_result.rs`: `TimeRange`, `SoundClassification` (label and confidence 0 to 1) and
  `SoundClassificationResult` (time range plus classifications sorted highest first; one entry per label, keeping its
  highest confidence; `classification_for`, `highest`, `above`).
- `springboard/ui/src/sound_analyzer.rs`:
  - `SoundRequest`: classifier identifier, window duration (default 3 s), overlap factor (0 to 0.99), optional
    known-classification filter. Invalid numbers fall back to defaults.
  - `SoundClassifier` trait: the caller plugs in a classifier. No label set or model ships here, so the request is
    partial.
  - `SoundResultsObserving` trait: `did_produce`, `did_fail`, `request_did_complete`.
  - `SoundAnalyzer`: `new(format)`, `add` (returns `RequestId`), `remove`, `remove_all_requests`, `analyze(&AnalyzerInput)`,
    `complete_analysis`. It mixes input to mono, cuts windows at the hop implied by the overlap, classifies each complete
    window and reports a time range on its frame clock. It reuses `analyzer_input::{AnalyzerInput, AudioFormat}` from
    wave 2. A format mismatch reports `InvalidFormat`. A classifier error stops that request.
  - `SoundAnalysisError`: the five official codes (`invalidModel`, `invalidFormat`, `invalidFile`, `operationFailed`,
    `unknownError`), with `code_name()` and `Display`.
- `springboard/ui/src/lib.rs`: two `pub mod` lines after `analyzer_input`.

## Official API names mirrored

From `https://developer.apple.com/documentation/soundanalysis` (DocC via `tools/parity/docc.py`):
`SNClassificationResult`, `SNClassification`, `SNClassifySoundRequest` (`windowDuration`, `overlapFactor`,
`knownClassifications`, `classifierIdentifier`), `SNResultsObserving` (`request(_:didProduce:)`,
`request(_:didFailWithError:)`, `requestDidComplete(_:)`), `SNAudioStreamAnalyzer` (`init(format:)`, `add`, `remove`,
`removeAllRequests`, `analyze`, `completeAnalysis`), `SNError.Code` (`invalidModel`, `invalidFormat`, `invalidFile`,
`operationFailed`, `unknownError`).

Differences from the official API: the observer is per request, so callbacks take no request argument. `analyze` takes
an `AnalyzerInput` whose start time gives the frame position, not a separate frame argument. The request takes no
`mlModel`. The error domain string (`SNErrorDomain`) is not on the public pages, so it is not mirrored.

## REA evidence (rendered official pages)

| Page | evidence_id |
|---|---|
| Sound Analysis (framework landing) | `ev_b2fe5cbb2d43b8414ad4a294a8bc12102307678c00eedf1f9fb3242f9cf85d20` |
| SNClassifySoundRequest | `ev_1d919806f3000a8b17491002b664c5d3ac8f843bf6806dd5da4c46fed64dd65c` |
| SNClassificationResult | `ev_1ba364e0917924d45cf996559463f39adf98375e4c6b81bcfeccaabb4155b4d6` |
| SNResultsObserving | `ev_dcc960919ff98f62176f14dbdbef6c229ce9ca3c0b6804d949092c87479a8ef2` |
| SNError.Code | `ev_05d262959819b8cd54730b7280dcffbfaf874f28381c3283a9d75f958c6e4232` |

## Matrix rows closed (framework `sound-analysis`)

These rows move from `gap` to `partial`. The lead updates the matrix centrally.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| sound-analysis | Built-in sound classifier request | partial | springboard/ui/src/sound_analyzer.rs (`SoundRequest`) |
| sound-analysis | Classification results | partial | springboard/ui/src/sound_result.rs (`SoundClassificationResult`) |
| sound-analysis | Results observer callbacks | partial | springboard/ui/src/sound_analyzer.rs (`SoundResultsObserving`) |
| sound-analysis | Errors | partial | springboard/ui/src/sound_analyzer.rs (`SoundAnalysisError`) |

## Still open

- No built-in label set (over 300 sounds) and no model runtime, so no request can name a built-in classifier yet.
- Custom Core ML classifier requests (`SNClassifySoundRequest(mlModel:)`) are not started.
- No file analyzer wrapper: the caller decodes the file and passes blocks, so `Audio file analyzer` stays `gap`.
- The stream analyzer is not wired to the audio service or the SDK, so the `Audio stream analyzer` row stays `os`.
- Live audio sample app: not started.
- The tests are unverified until the integration build runs.
