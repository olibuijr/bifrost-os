# Wave 4: Swift Charts parity (pure models)

Framework: Swift Charts, https://developer.apple.com/documentation/charts (macOS 13.0; 3D charts macOS 26.0).
Scope: the gap rows that pure Rust can cover in `bifrost-ui` (`springboard/ui/src`). The new modules are exposed to
apps as `bifrost_app::controls::<module>`, because the SDK re-exports `bifrost_ui` as `controls`.
Nothing here draws to a Surface yet and nothing is wired into the shell, so the Screen Time chart and the battery
graph still use their private drawers. The new code is covered by host unit tests only.

## What was built

- `springboard/ui/src/chart_content.rs`: `Chart`, a builder of `Mark`s (`Bar`, `Line`, `Area`, `Point`, `Rectangle`,
  `RuleX`, `RuleY`, `Sector`). It computes the automatic scale domains from the data (bars and areas include the
  baseline; a single value gets a unit-wide span) and `render` returns `Drawn` geometry (points, paths, polygons,
  rectangles, segments) inside a frame. Mirrors `Chart`, `ChartContentBuilder`, `ChartContent`, `Plot`,
  `AutomaticScaleDomain` and `BarMark`.
- `springboard/ui/src/chart_vector.rs`: batched plots. `decimate` keeps at most four points per plot column (first,
  last, lowest, highest), so a large series draws with bounded geometry. `bar_plot` returns one frame per bar in one
  pass. Mirrors `VectorizedChartContent`, `LinePlot` and `BarPlot`.
- `springboard/ui/src/chart_scroll.rs`: `ValueAligned`, which snaps a scroll target to values spaced at a unit, with an
  optional major value and a `Limit`. Mirrors `ChartScrollTargetBehavior.valueAligned(matching:majorAlignment:limitBehavior:)`
  for numeric values.
- `springboard/ui/src/lib.rs`: three `pub mod` lines.

## Differences from the reference

- `ValueAlignedLimitBehavior` has `always`, `automatic` and `never` in the official docs. Only `Never` and `Always` are
  built. The official page does not define the exact rule for `always`; `Always` here means "at most one unit from the
  start of one gesture", which is a BifrOSt definition and is not checked against Apple's behaviour. `automatic` is not built.
- `MajorValueAlignment` has `matching(_:)` (calendar components), `unit(_:)` and `page`. Only a numeric major unit is
  built, and it snaps only when the gesture ends within half a unit of a major multiple (a BifrOSt rule). Date components
  and `page` are not built.
- Batching in `chart_vector` is a geometry reduction, not Apple's vectorized content path. It is meant for the
  renderer; the decimation rule itself is BifrOSt's own.
- No 3D charts, no accessibility of marks, no pointer hit-testing, no chart drawing into a Surface.
- `Chart::domains` widens a single value by 0.5 on each side. The official docs do not describe this; it is a BifrOSt rule.

## Matrix rows closed (for the lead to merge)

| Framework | Feature (as in data/swift-charts.json) | New status | BifrOSt path |
|---|---|---|---|
| Swift Charts | Chart view and content builder | partial | springboard/ui/src/chart_content.rs |
| Swift Charts | Bar marks | partial | springboard/ui/src/chart_content.rs (Mark::Bar), springboard/ui/src/chart_vector.rs (bar_plot) |
| Swift Charts | Vectorized plots for large data | partial | springboard/ui/src/chart_vector.rs (decimate, bar_plot) |
| Swift Charts | Scrolling charts | partial | springboard/ui/src/chart_scroll.rs (ValueAligned) |

Bar marks stays partial because the geometry is app-facing through `bifrost_app::controls`, while the Screen Time chart and
the battery graph still draw their own bars.

## REA evidence

- `ev_710b4703fde20901e640eb13b99bcac31d324c9c46f2bb7637a446d828ec49dd`: Swift Charts landing page
  (https://developer.apple.com/documentation/charts), used for the marks, plots and scroll topics above.
- `ev_07669e5ca1e7b8c62621ff966ca53fbaaa9cf5c21068f71e20fec1e635ca7f89`: ChartScrollTargetBehavior
  (https://developer.apple.com/documentation/charts/chartscrolltargetbehavior), used for `valueAligned`,
  `MajorValueAlignment` and `ValueAlignedLimitBehavior`.

## Tests

Each new module has a `#[cfg(test)] mod tests` with hand-worked values: scroll snapping and limits, decimation of a
five-point series (expected output given by hand), bar rectangles from the plot scales, step-line paths, rectangle and
rule bounds, pie sector outlines, and automatic domains. The builds and the test run were not executed in this wave, as the
task requires.
