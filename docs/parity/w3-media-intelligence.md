# Parity wave 3: Media Intelligence

Framework: Media Intelligence, https://developer.apple.com/documentation/mediaintelligence (macOS 27.0).
Status before this wave: gap, 0.0% coverage.

## What was built

Three pure models in `springboard/ui/src` (bifrost-ui, std only, no I/O, no model). They cover the parts of the
analysis APIs that need no neural network. The on-device analysis itself (face detection, scoring frames with a
model) is not built, so the rows stay `partial`.

1. `springboard/ui/src/keyframe_pick.rs`: representative frame selection. `histogram(luma, bins)` builds a luma
   histogram. `representative(frames)` returns the frame that is closest to all others (the medoid), with its time.
   Mirrors `KeyFrameAnalysisRequest` (result: the frame's timestamp, for seeking or a thumbnail). Rule: histograms
   normalised to parts per million, distance = half the sum of absolute bin differences, minimum total distance wins,
   ties go to the earlier frame. This is a histogram rule, not a learned one.
2. `springboard/ui/src/highlight_rank.rs`: highlight segments. `highlights(scores, window, count)` ranks windows of
   `window` frames by mean per-frame score and keeps up to `count` non-overlapping windows, best first. Mirrors
   `HighlightAnalysisRequest` / `VideoAnalyzer` result segments. The per-frame scores come from the caller; this
   module does not measure engagement.
3. `springboard/ui/src/face_group.rs`: face grouping. `FaceGroups` stores face embeddings per image asset
   (`insert_face`, `delete_asset`, `delete_all`, `faces_in_asset`) and `entities(threshold)` groups faces by cosine
   similarity with single linkage (union-find). Mirrors `FaceGroupAnalyzer` (`insertOrUpdateAssets`, `deleteAssets`,
   `deleteAllAssets`, `fetchFaces(in:)`, entities of faces). There is no face detector: the embeddings come from the
   caller. There is no persistent working directory; the store is in memory.

Each module has a `#[cfg(test)] mod tests` with hand-worked values (6 tests per module). Modules are registered
with one `pub mod` line each at the end of `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `KeyFrameAnalysisRequest` (`init()`, `KeyFrameAnalysisRequest.Result`)
- `HighlightAnalysisRequest` (`init()`, `HighlightAnalysisRequest.Result`), `VideoAnalyzer`
- `FaceGroupAnalyzer` (`init(workingDirectory:)`, `insertOrUpdateAssets(_:)`, `deleteAssets(_:)`, `deleteAllAssets()`,
  `fetchFaces(in:)`, `allFaces`, `FaceGroupAnalyzer.Entity`, `FaceGroupAnalyzer.Face`)

Apple's own algorithms are not public. The rules above are BifrOSt's, so the behaviour is not identical.

## REA evidence

- Framework landing page (from the matrix): `ev_2c9b700223e1adc5dc69aeb5346e155855b4bfa0499c492ea61bf0ef6041038c`
- `KeyFrameAnalysisRequest`: `ev_9895b27dbbf6471c42a61716e9fbd5c0bc445bc859edd6c7d3b0cfcb9ba783f8`
- `HighlightAnalysisRequest`: `ev_9f43c15396af7eebbe79384c51b0670bcc667775be80fa54d34174f669508de4`
- `FaceGroupAnalyzer`: `ev_26a5195ffcbc0d49431e2e68452e1f70b2baddf87450356eff63afc6558cbd90`

DocC facts (`tools/parity/docc.py mediaintelligence/...`): the video analysis article is "Finding the best moments in
a video"; the face article is "Detecting and grouping faces in images". `FaceGroupAnalyzer` stores its data in a
working directory that persists between launches. `identifyFaces(in:)` matches faces without storing them.

## Matrix rows closed

| Framework | Feature (exact, from `docs/parity-matrix/data/media-intelligence.json`) | New status | BifrOSt path |
|---|---|---|---|
| Media Intelligence | Face grouping in images (FaceGroupAnalyzer, MediaIntelligenceImageAsset) | partial | springboard/ui/src/face_group.rs |
| Media Intelligence | Best moment and highlight segments in video (VideoAnalyzer, HighlightAnalysisRequest) | partial | springboard/ui/src/highlight_rank.rs |
| Media Intelligence | Keyframe selection for video theme (KeyFrameAnalysisRequest) | partial | springboard/ui/src/keyframe_pick.rs |

Still gaps after this wave: the on-device ML analysis runtime (no inference in `springboard/`), face detection,
persistent analyzer storage, and the `MediaIntelligenceError` type (it only matters once an analysis can fail).

## Verification

Not built or run, as the wave rules require. `python3 tools/check-names.py` exits 0 on the new and changed files.
