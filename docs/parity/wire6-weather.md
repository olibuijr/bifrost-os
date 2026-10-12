# Wire 6: Weather (1.5.0) adds the area mark to the temperature chart

Branch `wire6/weather`, app `springboard/apps/weather-desktop/` (crate `bifrost-weather`). Version 1.4.0 to 1.5.0 in
`Cargo.toml` and `manifest.toml`, with a plain-language `CHANGELOG.md` entry at the top. Not built in this round: the
lead runs the one build and the host tests. `springboard/Cargo.lock` is not edited here.

This round stayed inside the app folder (plus this note). The focus asked for more than the folder can reach, so the
gaps below are the real reason most of the focus is not wired.

## What is now wired

| Model (ui/src) | Used for | Where |
|---|---|---|
| `chart_marks::area_outline` idea, `chart_marks::line_path` (`Interpolation::Linear`) | The temperature chart's area fill under the hourly line: one 1-point column per step, from the line down to the plot's bottom edge | `src/insights.rs` `area_columns` (new, pure); `src/weather.rs` `draw_main_with` TEMPERATURE card, before the line segments |

`area_columns` is an app-local helper over the points `line_path` returns (the `chart_marks` model already exists). It
is not a copy of `area_outline`: the only polygon fill found is `springboard/src/shell/music/icons.rs` `fill_poly`, a
shell helper and not a kit call, so the area is drawn as columns with `fill_rrect`.

The chart reaches the user as before: open Weather, pick a place, scroll below the detail tiles to TEMPERATURE. The
line now sits on a soft white fill (alpha 40) that runs down to the bottom of the plot.

## How a user reaches it

1. Open Weather. Select a place with hourly data.
2. Scroll the forecast down to the TEMPERATURE card. The fill is under the white line and dots, above the grid rules.

## How to verify on Midget (the test PC)

1. After the lead's release: `bifrost pc midget ota` until `phase=accepted`, then
   `bifrost pc midget open-app is.olibuijr.weather.desktop`.
2. `bifrost pc midget swipe` up to the TEMPERATURE card. `bifrost pc midget screen /tmp/weather-chart.png`. The fill
   should be continuous from the first hour ("Now") to the last, with no gaps between columns, and its top edge should
   match the white line exactly.
3. `bifrost pc midget appearance light` and `dark`: the fill stays visible on the sky backdrop in both.
4. Host preview: `bifrost pc smoke weather-desktop --sizes 1200x780 --scale 1.5625` (and `--light`).

Host unit tests (for the lead): `cargo test -p bifrost-weather --offline` in `springboard/`. The new test is
`insights::tests::area_columns_follow_the_line_at_each_step`: the line `(0,10)-(10,20)-(20,0)` at step 5 gives
`(0,10) (5,15) (10,20) (15,10) (20,0)`; one point or a step of 0 gives an empty list. The compliance snapshots of the
weather screen change below the chart's top edge; the lead re-blesses.

## REA evidence

No new REA capture in this round. The official-documentation evidence for the rows this note touches is already in the
matrix and the earlier notes:

- `ev_640a3b252271e036de45d2e26c328a09c623b3a03e40a778617a658c969c3ffc`: WeatherKit overview (`docs/parity-matrix/data/weatherkit.json` `rea_evidence`).
- `ev_199a2b1fb9be1fa02315a088a6a31becd8a715018b8c85ddaad263486f5407a4`: WeatherError page (wire 2 note).
- `ev_a252fd7d81b20ac3d9bf55801c0fdb5a18164d55927227563c5e43c5382caef3`: WeatherAlert page (wire 5 note).

The Swift Charts area and line mark behaviour was not re-read in this round; the area fill is an unstyled fill under the
line, so no new reference page was needed for it.

## Remaining gaps (not wired, and why)

- **Pressure and pressure units (hPa, inHg)**: the forecast request does not ask for pressure, and the pressure
  trend model (`ui/src/pressure_trend.rs`) has no data to classify. The shell's data module
  (`springboard/src/shell/weather/data.rs`) is outside this app folder, so the request cannot be changed here. No
  pressure tile and no pressure unit were added.
- **Visibility and precipitation tiles** (amounts, probability, snowfall): the forecast request asks for neither. The
  precipitation and minute models in `ui/src/precipitation.rs` have no data. Same fix, in the shell's data module.
- **Speed units**: the wind tile follows the C/F switch (km/h with Celsius, mph with Fahrenheit), as in wire 2. A
  separate speed setting needs `Measurement` in `springboard/sdk/src/locale.rs` to have a speed unit, and a persisted
  setting, both outside this folder. Not added.
- **Units in Settings**: the Locations screen holds the only unit switch (the toolbar C/F). No speed or pressure
  control was added, for the reasons above.
- **Wind direction and gust**: not requested; `Wind::compass` and `gust_factor` stay unused.
- **Hourly UV**: the forecast carries a daily peak only; the UV tile uses today's value.
- **Alerts banner**: still not drawn (no alert source; see the wire 5 note).

## Matrix rows touched

Rows are not edited here (the matrix is shared). The lead updates them in the same round as the release.

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| swift-charts | Line and area marks | `springboard/apps/weather-desktop/src/insights.rs` `area_columns`; `src/weather.rs` `draw_main_with` TEMPERATURE card |
| weatherkit | Current conditions | unchanged: `springboard/apps/weather-desktop/src/weather.rs` detail tiles |
| weatherkit | Wind | unchanged: `springboard/apps/weather-desktop/src/weather.rs` `wind_text` (speed only) |
| weatherkit | UV index | unchanged: `springboard/apps/weather-desktop/src/weather.rs` UV INDEX tile (daily peak) |
| weatherkit | Hourly and daily forecasts | unchanged: `springboard/apps/weather-desktop/src/weather.rs` `HOURLY_TOP`, `DAILY_TOP` |
| weatherkit | Pressure trend | not wired: needs pressure in the forecast request (shell data module) |
| weatherkit | Precipitation | not wired: needs amounts in the forecast request (shell data module) |
| weatherkit | Minute forecast | not wired: needs minute data (no source) |
