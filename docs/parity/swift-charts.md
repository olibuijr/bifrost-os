# Swift Charts parity: pure chart geometry

Status: partial. Official reference: https://developer.apple.com/documentation/charts (macOS 13.0+; 3D charts macOS 26.0+).

## What was built

`springboard/ui/src/chart.rs` (crate `bifrost-ui`, std only, no unsafe). It holds the numeric models behind four
parts of the framework. It has no drawing code and no SDK view yet.

| Official API | BifrOSt function | Behaviour |
|---|---|---|
| ScaleDomain, ScaleRange, PositionScaleRange | `LinearScale` (`new`, `map`, `invert`) | Linear map from a data domain to a range. Values outside the domain are not clamped. A reversed range flips the axis. A degenerate domain maps to the range start. |
| AxisMarks, AxisValue | `axis_ticks(lo, hi, count)` | Evenly spaced values with a step of 1, 2 or 5 times a power of ten. Values print exactly (no float noise). |
| NumberBins, ChartBinRange | `number_bins(values, count)` returning `Bin { lower, upper, count }` | Equal-width bins. The maximum falls in the last bin. Non-finite values are ignored. A single repeated value gets one unit-wide bin. |
| SectorMark, SectorPlot | `sectors(values, start)` returning `Sector { start, end }`, and `point(center, radius, angle)` | Clockwise sector spans from straight up, one per value, sized by value. Zero, negative and non-finite values take no angle. |

Tests: `cargo test` inside `springboard/ui` (not run by this stage). The module has 8 behaviour tests.

The new module is declared once in `springboard/ui/src/lib.rs` (`pub mod chart;`).

## REA evidence

- Landing page `https://developer.apple.com/documentation/charts`, captured with the local CDP browser:
  `ev_4b5d071ab2fd234fc97f2a909a12161f23133f73cdbd06844ee77f310a447687`. The rendered headings list the same
  Topics sections as the DocC index (Charts, 3D charts, Marks, Vectorized plots, Mark configuration, Labeled data,
  Scales, Axes, Axis marks, Annotations, Data bins, Chart management, Scrolling, Structures).
- Earlier matrix row evidence, kept for reference: `ev_a5b6db6d9b764459a732983f9d4019c663292a67875e4108d599e2a55b142ec9`.
- Official structure read from the DocC JSON for `charts` (`developer.apple.com/tutorials/data/documentation/charts.json`).

## Remaining gaps

- No app-facing chart view or mark API in `springboard/sdk/src`. Apps still hand-draw charts.
- The Screen Time and battery charts keep their own drawing code and do not call `chart.rs`.
- Not built: line, area, point, rectangle, rule and sector drawing; interpolation methods; vectorized plots.
- Scales: no automatic domain, no log or sqrt scale.
- Bins: no `DateBins`.
- Axes: no grid lines, tick styling or label API.
- No annotations, no chart proxy, no chart scroll behaviour.
- No 3D charts or surface plots (macOS 26.0 in Apple's docs).
- No chart accessibility summaries or localized axis labels.

The matrix row lives in `docs/parity-matrix/data/swift-charts.json`. That path is not on the base this branch was
cut from; the matrix branch `docs/apple-parity-matrix` holds the other rows.
