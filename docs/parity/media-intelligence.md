# Media Intelligence parity (sdk-rest-10)

## What it mirrors

The reference framework reports why an analysis request failed with one error type. BifrOSt has no app-facing analysis
request yet, so the error type covers the face grouping model and the shared analysis runtime.

## What is wired

- `springboard/ui/src/media_intelligence_error.rs`: `MediaIntelligenceError` (empty embedding, non-finite value,
  dimension mismatch, invalid threshold, runtime failure), `check_embedding` and `check_threshold`.
- `springboard/ui/src/face_group.rs`: `FaceGroups::try_insert_face` and `FaceGroups::try_entities`, which run the
  checks before storing a face or grouping entities. The unchecked `insert_face` and `entities` are unchanged.
- Tests: `springboard/ui/tests/parity_sdk-rest-10.rs`, headless, on plain values.

## What needs the lab machine

- An app-facing analysis request that returns the error to an app. It needs the on-device model runtime exposed to
  the SDK and a face detector, which do not exist yet.
