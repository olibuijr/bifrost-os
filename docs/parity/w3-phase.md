# Wave 3: PHASE gap rows (pure Rust models)

Framework: PHASE (spatial and game audio), official doc https://developer.apple.com/documentation/phase.
Branch `parity/w3-phase`, based on `parity/integration`. No builds were run (wave rule); the tests
are written, not executed.

## What was built

- `springboard/ui/src/sound_distance.rs` (bifrost-ui, pure model):
  - `GeometricSpreading`: distance attenuation with a min distance, a max distance and a roll-off
    factor. Gain is `min / (min + rolloff * (d - min))`, with `d` clamped to `[min, max]`.
  - `envelope_gain`: reads a gain from a distance envelope, clamped to 0..=1.
  - `cardioid_gain`: heart-shaped emission, `0.5 * (1 + cos(angle))`.
  - `Cone`: full level inside an inner half-angle, a set outer gain at or beyond the outer half-angle,
    linear between.
- `springboard/ui/src/sound_envelope.rs` (bifrost-ui, pure model):
  - `Curve`: segment shapes linear, squared, inverse squared, cubed, inverse cubed, sine,
    inverse sine, hold start and jump to end. Sigmoid and inverse sigmoid are not built.
  - `Segment` and `Envelope`: a start point and segments, with `evaluate`, `domain` and `range`.
    Input outside the domain is clamped. A zero-length segment jumps to its target.

Both modules are registered in `springboard/ui/src/lib.rs` (one `pub mod` line each).

## Official API names mirrored

- `PHASEGeometricSpreadingDistanceModelParameters` (`rolloffFactor`): `GeometricSpreading`.
- `PHASEEnvelopeDistanceModelParameters`: `envelope_gain` over `Envelope`.
- `PHASECardioidDirectivityModelParameters`: `cardioid_gain`.
- `PHASEConeDirectivityModelParameters`: `Cone`. Subband parameters are not modelled.
- `PHASEEnvelope` (`init(startPoint:segments:)`, `evaluate(x:)`, `segments`, `startPoint`, `domain`, `range`): `Envelope`.
- `PHASEEnvelopeSegment`: `Segment`.
- `PHASECurveType` (linear, squared, inverseSquared, cubed, inverseCubed, sine, inverseSine, holdStartValue,
  jumpToEndValue): `Curve`. `sigmoid` and `inverseSigmoid` are missing.

Official pages read with `docc.py`: `phase/phasegeometricspreadingdistancemodelparameters`, `phase/phaseenvelope`,
`phase/phasecurvetype`, `phase/phasecardioiddirectivitymodelparameters`, `phase/phaseconedirectivitymodelparameters`.
The curve and cone formulas are BifrOSt's own rules, not Apple's internal ones. The docs do not publish them.

## REA evidence

- `ev_fa11407b7cf72a5613863c1f7d4f3fa0eb0bf1c86aafed8bd8ca5eb697af2b5f` (PHASE landing page)
- `ev_7741b088c9a12b6c252492430cf6e8369aacbcbe2d9984aba38e9651a01aa81b` (PHASEGeometricSpreadingDistanceModelParameters)
- `ev_b87dd2e223e25134f0771af2d1fd5711752d7bdea4e347d6ff6b7da3547d46ba` (PHASEEnvelope)
- `ev_fa9dc9e435fd9cb2f1b7df75276dca79777cd763ae3aae13f9c997318c97a02f` (PHASECurveType)
- `ev_65d153b49ac9d49b49ea4ae3ec5872cfd3ccc2b235d99d0fa6c56ec8a1769394` (PHASECardioidDirectivityModelParameters)

## Matrix rows closed

| framework | feature | status | bifrost |
|---|---|---|---|
| PHASE | Distance models and directivity | partial | springboard/ui/src/sound_distance.rs |
| PHASE | Runtime parameters and envelopes | partial | springboard/ui/src/sound_envelope.rs |

Still open in these rows: no sound source or listener position (`Spatial sources and listener` stays gap),
no meta parameters or event graph, no engine to apply the gains, and no SDK exposure. Sigmoid curves
and cone subbands are missing.

## Tests

- `sound_envelope.rs`: 9 tests (linear, squared, inverse sine at 0.5, two-segment join, hold and jump,
  domain clamp, range, zero-length segment, invalid input).
- `sound_distance.rs`: 9 tests (inside minimum, roll-off value 1/3, max-distance hold 0.1, zero roll-off,
  invalid spreading, envelope gain with clamp, cardioid points, cone interpolation 0.625, invalid cone).
