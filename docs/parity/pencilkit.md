# PencilKit parity, team J

Two gap rows of the PencilKit framework are now modelled in pure Rust in `bifrost-ui`, and the stroke-level eraser
model already in `drawing.rs` is recorded against its row. Each module is data and geometry only: reading the input
device, drawing strokes and drawing the tool palette still belong to the shell and apps. Nothing here is wired into
the shell or an app yet, and nothing has been built or run on Midget. The rows are built on branch
`parity/team-j-pencilkit`; not yet verified on Midget.

## What was built

- `springboard/ui/src/handwriting.rs` (`pub mod handwriting`):
  - `segment_lines(&Drawing) -> Vec<Line>` groups strokes into text lines. A stroke joins a line when its box overlaps
    the line's vertical band by at least half of the smaller height. Lines read top to bottom; strokes in a line read
    left to right.
  - `Recognizer` trait: `recognize(&[&Stroke]) -> Option<String>`. The shell or an app supplies the implementation.
  - `recognize_lines(&Drawing, &dyn Recognizer) -> Vec<(Rect, String)>` runs the recogniser once per line and keeps
    the lines it reads.
- `springboard/ui/src/scribble.rs` (`pub mod scribble`):
  - `classify(&Stroke) -> Option<Gesture>`: `Tap` when the longest side is at most 12 points; `ScratchOut` when the
    stroke turns back on itself at least three times along one axis and travels at least three times its extent;
    `Write` otherwise. Steps under 2 points count as jitter.
  - `resolve(&Stroke, &[(u32, Rect)]) -> Action`: a tap selects the topmost view under it; a scratch-out deletes
    every view its points cross, bottom first; writing goes to the topmost view under the centre of the stroke.
- `springboard/ui/src/drawing.rs` already has `Drawing::erase_strokes_at`, a stroke-level eraser. It is now named in
  the eraser row of the matrix.

## Matrix rows

`docs/parity-matrix/data/pencilkit.json`:

| Feature | Previous status | New status |
|---|---|---|
| Handwriting recognition (PKStrokeRecognizer) | gap | partial (layout and hook only; no recogniser) |
| Scribble on non-text views | gap | partial (gesture model only) |

The summary and gaps fields are updated to match. The framework stays `partial`.

## Still open

- A handwriting recogniser. Letter recognition needs a model or template library that is not in the tree; the
  `Recognizer` trait is the seam for it.
- Routing stylus strokes into text fields and other views, and the rendering of Scribble feedback.
- Using the ui drawing model in the shell markup editor and Freeform, which still keep their own stroke code.

## Tests

- `scribble.rs`: 12 tests, covering tap, writing and scratch-out classification, jitter, the two-reversal boundary,
  topmost tap hit, scratch-out target order, writing target choice and empty inputs.
- `handwriting.rs`: 6 tests, covering line grouping, reading order of lines and strokes, the overlap threshold,
  strokes with no points, an empty drawing, and the recogniser hook.

Tests have not been run in this round, as the build rules for this round forbid it. The lead runs them.
