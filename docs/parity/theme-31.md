# Theme 31: accessibility and localization of charts

Theme size 13. This round closes three matrix rows with one shared module, `springboard/ui/src/chart_ax.rs`
(`bifrost_ui::chart_ax`). It is a pure model: no speech, no sound output and no drawing happens in it.

## What was built

- `ChartDescriptor`: title, summary, a categorical or numeric x axis, an optional numeric y axis and a list of
  data series. It mirrors the semantic description a chart hands to the accessibility layer.
- Point descriptions: `describe` speaks a point as label (with its place among the categories), series name and
  value with its unit. A missing value is spoken as "no value" and is never read as zero.
- Overview: `overview` speaks the title, summary and size of the chart.
- Rotor-style navigation: `step` moves a `Cursor` by point (next, previous, first, last) or by series, keeping
  the point index clamped to the target series. It returns `None` at an edge.
- Audio graph: `audio_graph` turns one series into `Tone`s. Pitch follows the y value on a logarithmic scale
  between 220 Hz and 1760 Hz. Tone length follows the first extra value between 100 ms and 800 ms. Timbre is the
  series index. A missing value becomes a rest (0 Hz) and keeps its place and default length.
- Localization: `Separators` and `format_number` write numbers with the locale's decimal and grouping
  characters (English "1,234.5", Icelandic "1.234,5"). `Separators::for_language` picks them from a tag.

## Apple APIs mirrored

Official pages, read with `tools/parity/docc.py` and `tools/parity/rea-page.sh`:

- `AXChartDescriptor` (title, summary, xAxis, yAxis, additionalAxes, series, contentFrame):
  https://developer.apple.com/documentation/accessibility/axchartdescriptor
- `AXDataSeriesDescriptor` (name, isContinuous, dataPoints), `AXDataPoint` (x, y, additionalValues, label),
  `AXNumericDataAxisDescriptor` (range, value description provider), `AXCategoricalDataAxisDescriptor`
  (categoryOrder): https://developer.apple.com/documentation/accessibility/axdatapoint
- Audio graphs article: pitch from the numeric axis, tone length from an additional value, timbre from the
  series. https://developer.apple.com/documentation/accessibility/representing-chart-data-as-an-audio-graph
- Swift Charts landing page, whose accessibility and localization row this closes:
  https://developer.apple.com/documentation/charts

Not mirrored: `additionalAxes` beyond x and y, `contentFrame` and `contentDirection`, attributed titles, and
the `AXChart` protocol. The model has no view to attach to.

## REA evidence

- `ev_edfd4db208d8ae4dd6a5bd4b6f8b450a218b7b40a976775cc63f714e51f1d20f`: rendered page of `AXChartDescriptor`.
- `ev_b757e3980e48ff734472d9dbae60b661dd4f758feb2d88573b49e3d2cda0d7a4`: rendered page of "Representing chart
  data as an audio graph".

## Matrix rows closed

| Framework slug | Feature (as in data file) | New status | BifrOSt path |
|---|---|---|---|
| swift-charts | Accessibility and localization of charts | partial | springboard/ui/src/chart_ax.rs |
| accessibility | Audio graphs and hearing device support | partial | springboard/ui/src/chart_ax.rs (`ChartDescriptor::audio_graph`) |
| swiftui-events-a11y-integration | Accessible navigation (rotors) | partial | springboard/ui/src/chart_ax.rs (`ChartDescriptor::step`) |

## Remaining gaps

- No chart view or renderer uses the model, so the shell bars and apps still have no accessible values.
  The Swift Charts container and marks rows stay gap.
- Nothing plays the audio graph. There is no audio output path and no hearing device support.
- The rotor is chart-only. A general rotor for lists and text, and the accessibility tree hookup that would
  deliver the steps to a screen reader, are still missing.
- The announcement path is not wired: `describe` returns text, but it is not posted to `ax::Announcements`.
- Widget accessibility descriptions (WidgetKit row) are not closed by this round.

## Tests

`cargo` was not run in this round (no-build rule). The module carries 9 behaviour tests in `mod tests`:
number separators and the language map, point descriptions with units and category place, overview text, cursor
steps and edges, cursor movement between series with point clamping, logarithmic pitch and its midpoint, rests
for missing values, tone lengths from extra values, unknown-series handling, and grouping off.
