# Theme 25: point, rectangle and rule marks, and signposts

Branch `parity/theme-25`. Pure Rust, std only, no unsafe, no new crates. Nothing was built or run for this round
(the round rules forbid builds); the tests are written but not executed.

## What was built

### Chart mark geometry: `springboard/ui/src/chart_marks.rs` (`bifrost_ui::chart_marks`)

Builds on the existing `chart.rs` (`LinearScale`, `Sector`, `point`). It adds:

- `Frame` and `Plot`: the plot area and its x and y scales. `Plot::position` maps data values to screen points
  (chart proxy `position(for:)`), `Plot::value` maps back (`value(at:)`). The y scale runs upward, so larger values sit higher.
- Point marks: `point_marks` maps pairs to points and skips non-finite pairs.
- Rectangle marks: `rectangle_mark` returns the normalised frame for a pair of x and y bounds, or None when a bound is not finite.
- Rule marks: `vertical_rule` and `horizontal_rule` return a `Segment`.
- Line and area marks: `line_path` with `Interpolation::{Linear, StepStart, StepEnd, StepCenter}`, which sorts by x and
  drops invalid pairs. `area_outline` closes the line on a data-value baseline.
- Sector outlines: `sector_outline` gives the outer arc and the inner arc of a pie or donut sector with an angular inset.
- Annotations: `place_annotation` with `Position` (Automatic, Top, Bottom, Leading, Trailing, the four corners, Overlay)
  and `Overflow::{Fit, Disabled}`. `Fit` slides the annotation inside the plot; an annotation larger than the plot is pinned to its start edge.

Not built: monotone, cardinal and Catmull-Rom interpolation; the 3D marks; vectorised plots; the scrolling target
behaviour; the Apple per-axis overflow strategies (`AnnotationOverflowResolution`); and the rendering of these
paths. Nothing draws them yet.

### Signpost bookkeeping: `springboard/core/src/signpost.rs` (`bifrost_core::signpost`)

- `Id`: unique within a signposter (`Signposter::make_id`), or stable across processes from a text key (`Id::from_key`, FNV-1a).
- `Signposter`: `begin` returns an `Open` token, and `end` consumes it, so an interval ends at most once. `end` refuses an
  interval from another signposter (`ForeignInterval`), a different name (`NameMismatch`) and a reversed clock
  (`TimeReversed`). `disabled()` accepts the same calls and records nothing. `event` returns a point of interest.
- Times are nanoseconds supplied by the caller, so the module reads no clock.
- `Summary`: per-name `Stats { count, total_ns, max_ns }` over finished intervals, with `take()` to deliver once.

Not built: histogram buckets, the daily report delivery and the animation-interval variant.

### App-facing signposts: `springboard/sdk/src/signpost.rs` (`bifrost_app::signpost`)

- `Signposter::new(subsystem, category)` and `disabled(...)`, wrapping the core signposter and a unified-log `Logger`.
- `begin`, `end` (logs the finished interval at info level), `event` (logs at info level) and `measure`, which
  runs a closure as one interval and returns its value.
- Re-exports `Error`, `Event`, `Id`, `Interval`, `Open`, `Stats` and `Summary`.

Not built: a reader for log entries of signpost type, and the profiler track. Signposts reach the Console app as
ordinary info lines, not as a separate entry type.

## Official API names mirrored

| Apple page (developer.apple.com) | Mirrored by | REA evidence |
|---|---|---|
| `charts/pointmark` (PointMark) | `chart_marks::point_marks` | ev_dbb9c3b8ab1f589a7fe4ffefab15b2db89b9b330d792180362f55ba02fe22493 |
| `charts/rectanglemark` (RectangleMark) | `chart_marks::rectangle_mark` | ev_181117f00a040eede51ffd1c071c5bad50d5db6f77ef627c654efd9a06614838 |
| `charts/rulemark` (RuleMark) | `chart_marks::vertical_rule`, `horizontal_rule` | ev_f588fc89e82221e1e6ba888658b6f60a55e7e6216227936f95b5b73ab72f5a7c |
| `charts/annotationposition` (AnnotationPosition) | `chart_marks::Position`, `place_annotation` | ev_d0ce5d6f988e058e35a8f970c1f89f46c81ae3f1c1b9dc704e91f1ade290c2e5 |
| `charts/sectormark` (SectorMark) | `chart_marks::sector_outline` | ev_0aa24d36fac60590e2bf3a9c9d7ca8be0331ed9bceee9e723454555ad1df58d8 |
| `charts/interpolationmethod` (InterpolationMethod) | `chart_marks::Interpolation` (linear and step only) | ev_b852e12c8d5ca4ee4ee46a55051703d0698b2b07b4c7347bee323189a4de9bd7 |
| `charts/chartproxy` (ChartProxy) | `chart_marks::Plot::position`, `value` | ev_e02f0af795c7b593f092f87e7982dab11db2d83f73f068edb82152c369809305 |
| `os/ossignposter` (OSSignposter) | `core::signpost::Signposter`, `sdk::signpost::Signposter` | ev_57d6104f9352ce612c756d830f8e9fd4f1bdcd64b3d57794eb4be9a09d4856b5 |

Official facts used from the DocC pages (`tools/parity/docc.py`): PointMark, RectangleMark and RuleMark are the
macOS 13 mark types; AnnotationPosition has ten positions, including `automatic` and `overlay`; SectorMark is macOS 14;
InterpolationMethod has seven methods; ChartProxy has `position(for:)`, `position(forX:)` and `value(at:as:)`; OSSignposter
has `beginInterval`, `endInterval`, `emitEvent` and `makeSignpostID`, and one begin and one end per interval.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | Status | BifrOSt path |
|---|---|---|---|
| swift-charts | Point, rectangle and rule marks | partial | springboard/ui/src/chart_marks.rs |
| swift-charts | Annotations | partial | springboard/ui/src/chart_marks.rs (`place_annotation`) |
| swift-charts | Line and area marks | partial | springboard/ui/src/chart_marks.rs (`line_path`, `area_outline`; linear and step only) |
| swift-charts | Sector marks (pie and donut) | partial | springboard/ui/src/chart_marks.rs (`sector_outline`); springboard/ui/src/chart.rs (`sectors`) |
| swift-charts | Chart proxy and plot area access | partial | springboard/ui/src/chart_marks.rs (`Plot`) |
| os | Signposts (OSSignposter, os_signpost) | partial | springboard/core/src/signpost.rs; springboard/sdk/src/signpost.rs |
| oslog | Signpost entries and OSSignposter | partial | springboard/sdk/src/signpost.rs (logs to the unified log; no entry reader) |
| metrickit | Custom signpost intervals (SignpostIntervalMetric, mxSignpost) | partial | springboard/core/src/signpost.rs (`Summary`; no daily delivery) |

Not closed: the statereporting row "Points of Interest and signposts for Instruments" (no profiler track exists).

## Tests

- `springboard/ui/src/chart_marks.rs`: 11 tests (plot mapping and inverse, point marks skipping invalid pairs,
  rectangle normalisation, rules, linear join order, step methods, area baseline, sector radii and inset, annotation sides,
  annotation fit and overflow, automatic side choice).
- `springboard/core/src/signpost.rs`: 8 tests (stable and distinct ids, unique `make_id`, end-once and name check,
  foreign interval, reversed clock, disabled signposter, events, summary totals).
- `springboard/sdk/src/signpost.rs`: 5 tests (measure value, disabled end, name mismatch, distinct ids, enabled interval).

Commands for the lead to run: `cargo test --offline -p bifrost-ui chart_marks`, `cargo test --offline -p bifrost-core signpost`,
and the sdk crate's `signpost` tests. The owner name guard `python3 tools/check-names.py` passes (exit 0).
