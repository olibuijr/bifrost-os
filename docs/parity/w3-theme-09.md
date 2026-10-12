# Wave 3, theme-09: Instrument activity (sound analysis, spatial audio, pace)

Branch `parity/w3-theme-09`, built on `parity/integration` (waves 1 and 2). Pure Rust, std only, no unsafe, no new
crates. All code is in the `bifrost-ui` crate (`springboard/ui/src`), the pure-model layer. The code was not built or
run in this round (no builds, per the wave rules), so the tests listed below are written and hand-checked, not run.

## What was built

| Module | Status | What it models |
|---|---|---|
| `springboard/ui/src/sound_classify.rs` (new) | pure model | Classification results (top candidates per time span), classification requests (window duration, overlap, known labels), a file analyzer with observer callback and cancel, and an error set |
| `springboard/ui/src/sound_envelope.rs` (new) | pure model | Envelopes built from segments over a linear input, with eleven curve types, domain and range, and clamped evaluation |
| `springboard/ui/src/sound_distance.rs` (new) | pure model | Geometric spreading distance model, envelope distance model, cardioid directivity and cone directivity |
| `springboard/ui/src/sound_event.rs` (new) | pure model | Sound event trees: container, switch, weighted random and blend nodes, returning the audible leaves with gains |
| `springboard/ui/src/music.rs` (extended) | pure model | `PaceResult` and `analyze_pace`, the events-per-minute rate of each window of beats. Not added to `analyze()` or `SUPPORTED` |
| `springboard/ui/src/lib.rs` | 4 lines | `pub mod` for the four new modules |

The models are not wired into the shell or any app. They have no audio device, no decoding and no framework
dependency. The caller supplies the classifier, the random roll and the blend value.

## Official API names mirrored

Sound Analysis (`soundanalysis`): `SNClassificationResult` (`timeRange`, `classifications`, `classification(forIdentifier:)`),
`SNClassification`, `SNClassifySoundRequest` (`init(classifierIdentifier:)`, `init(mlModel:)`, `windowDuration`,
`overlapFactor`, `knownClassifications`), `SNAudioFileAnalyzer` (`analyze()`, `add(_:withObserver:)`, `cancelAnalysis()`),
`SNError.Code` (`invalidModel`, `invalidFormat`, `invalidFile`, `operationFailed`, `unknownError`).
PHASE (`phase`): `PHASEEnvelope` (`startPoint`, `segments`, `evaluate(x:)`, `domain`, `range`), `PHASECurveType` (eleven cases),
`PHASEGeometricSpreadingDistanceModelParameters` (`rolloffFactor`), `PHASEEnvelopeDistanceModelParameters` (`envelope`),
`PHASECardioidDirectivityModelParameters` and `PHASEConeDirectivityModelParameters` (the cone angles are a BifrOSt
parameterisation, the official classes take subband parameters), `PHASESoundEventNodeDefinition` with the switch,
random and blend subclasses (`addSubtree(_:switchValue:)`, `addSubtree(_:weight:)`, `addRange(envelope:subtree:)`).
Music Understanding (`musicunderstanding`): `PaceResult` (`ranges`).

Names in the code are BifrOSt's own (`SoundError`, `CurveType`, `EventNode`, `GeometricSpreading`, `ConeDirectivity`,
`analyze_pace`); the framework prefixes are kept out of the `.rs` files by the owner rule. Mapping:
`SoundError` = `SNError.Code`, `ClassificationResult` = `SNClassificationResult`, `CurveType` = `PHASECurveType`,
`Envelope` = `PHASEEnvelope`, `EventNode` = `PHASESoundEventNodeDefinition` and its subclasses.

## Decisions the reference does not dictate

- Curve formulas are BifrOSt's choice. The reference names the curve types but does not publish their maths. The
  inverse of every curve is `1 - f(1 - t)`. Sigmoid is smoothstep. Hold and jump return the start and end value.
- The reference's segment and meta-parameter fields are not all mirrored. A segment here is an end point and a curve.
- `TOP_CANDIDATES` is 3. The reference keeps "top candidates" without a fixed count here.
- Windows that run past the buffer end are not analysed. A buffer shorter than one window gives no results.
- `Cancelled` and `UnknownError` are in `SoundError`. `Cancelled` is a BifrOSt addition; `UnknownError` is reserved and
  never produced.
- `analyze_pace` counts beats per window. The reference's pace is a perceptual energy measure, so the two agree only
  on steady tempo.
- No built-in label set exists. Without a classifier that the app supplies, classification cannot run. There is no
  Core ML runtime, no subband directivity and no ducking.

## Matrix rows closed (status partial)

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| sound-analysis | Classification results | partial | `springboard/ui/src/sound_classify.rs` |
| sound-analysis | Built-in sound classifier request | partial | `springboard/ui/src/sound_classify.rs` (`ClassifyRequest`; no built-in model) |
| sound-analysis | Custom Core ML sound classifier request | partial | `springboard/ui/src/sound_classify.rs` (caller-supplied classifier; no Core ML) |
| sound-analysis | Audio file analyzer | partial | `springboard/ui/src/sound_classify.rs` (`SoundFileAnalyzer`; sync, buffer input only) |
| sound-analysis | Errors | partial | `springboard/ui/src/sound_classify.rs` (`SoundError`) |
| phase | Sound event nodes (switch, random, blend, container) | partial | `springboard/ui/src/sound_event.rs` |
| phase | Runtime parameters and envelopes | partial | `springboard/ui/src/sound_envelope.rs` (envelopes only; no meta parameters) |
| phase | Distance models and directivity | partial | `springboard/ui/src/sound_distance.rs` (no subbands) |
| music-understanding | Pace | partial | `springboard/ui/src/music.rs` (`analyze_pace`; not in `analyze()`) |

Not closed in this theme: Instrument activity (needs source separation), Live audio classification sample, Groups and
ducking, Spatial sources and listener, Occluders, Core Motion raw events, Force Feedback effects, Media Intelligence
errors, Audio Toolbox voice I/O and scheduled file regions, and the hardware clock rows.

## Tests (written, not run in this round)

- `sound_classify`: request validation; window and hop sizes; overlapping windows with time spans and classification
  lookup; candidate ordering with tie-break and truncation; invalid confidence fails; cancel from the observer; buffer
  shorter than a window.
- `sound_envelope`: linear ramp and clamping; power curves at one half; sigmoid and sine closed forms; hold and jump;
  two segments meeting; refused envelopes.
- `sound_distance`: geometric spreading at roll-off 1, 2 and 0.5; refused parameters; envelope distance clamping;
  cardioid pattern; cone holding, fading and clamping; cone angle checks.
- `sound_event`: container order; switch match and miss; random weight cut and last-roll fallback; blend gains; gains
  multiplying down the tree and silent branches dropped.
- `music::pace_tests`: steady beats; a slower stretch; trailing part dropped; too few beats, zero window, zero span.

## REA evidence (official rendered pages, 2026-10-11)

| Page | REA evidence id |
|---|---|
| soundanalysis/snerror | `ev_03f770785e6832fab83fb09abdad59505dcf0b8b74690fc4e542500049242b29` |
| soundanalysis/snclassificationresult | `ev_e79e0411c828c2c5efc33821d18caecc2589b1d192093d30d0347ca573999487` |
| phase/phaseenvelope | `ev_534c06067515bcd00d7800e31f4cdbf6cf7f774bffa7fba8e66e122276a8d84b` |
| phase/phasegeometricspreadingdistancemodelparameters | `ev_899df2a30e2d532f808a5c361081d7c4c576abfee4e1b1f2bfbda962110a731a` |
| musicunderstanding/paceresult | `ev_6931d4751dd7b5add4f3a28adb8fb49ed29971f978a581b1927fd1f73b0ed851` |

DocC (`python3 -I tools/parity/docc.py`) was also read for the Sound Analysis classes, the PHASE sound event nodes,
the curve type list, the cardioid and cone classes and the ducker and group classes.

## Checks

- `python3 tools/check-names.py`: exit 0 (no hits).
- No `Cargo.toml`, `CHANGELOG`, version or shared doc edited. `docs/parity-matrix/` not edited.
