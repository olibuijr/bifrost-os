# Theme 37: Precipitation (WeatherKit, WidgetKit)

## What was built

Three pure Rust models in `springboard/ui/src` (bifrost-ui, std only, no I/O), each with its own tests:

- `springboard/ui/src/precipitation.rs`: `Precipitation` (none, rain, snow, sleet, hail, mixed), `PrecipitationAmountByType`
  (rain, snow, sleet and hail amounts, total, dominant form), `SnowfallAmount`, `MinuteWeather` (one minute, with chance
  and intensity), and `MinuteForecast` (time-ordered minutes, `current`, `next_change`, `peak`, and a one-line `summary`
  such as "Rain starts in 12 min").
- `springboard/ui/src/weather_alert.rs`: `Severity` (unknown, minor, moderate, severe, extreme; ordered, with official
  threat descriptions), `WeatherAlert` (summary, region, severity, source, safe details link), `sorted` and `most_severe`,
  and `WeatherAvailability` (whether alerts and the minute forecast are served for a location).
- `springboard/ui/src/weather_attribution.rs`: `WeatherAttribution` (provider name, https-only legal page, legal text and
  the credit line the shell prints under provider data).

All three modules are registered in `springboard/ui/src/lib.rs` with one `pub mod` line each.

Every constructor clamps bad input (negative, NaN or infinite amounts, out-of-range chances, non-https legal links), so a
broken data feed cannot show a negative amount or open an untrusted link.

## Official API names mirrored

Official pages read with `tools/parity/docc.py` (DocC JSON):

- `weatherkit/precipitation`: `Precipitation` with `none`, `rain`, `snow`, `sleet`, `hail`, `mixed`, plus `description`.
- `weatherkit/weatheralert`: `WeatherAlert` with `metadata`, `region`, `severity`, `summary`, `detailsURL`, `source`.
- `weatherkit/weatherseverity`: `WeatherSeverity` with `minor`, `moderate`, `severe`, `extreme`, `unknown`.
- `weatherkit/weatheravailability`: `WeatherAvailability` with `alertAvailability`, `minuteAvailability`.
- `weatherkit/minuteweather`: `MinuteWeather` with `date`, `precipitation`, `precipitationChance`, `precipitationIntensity`.
- `weatherkit/weatherattribution`: `WeatherAttribution` with `serviceName`, `legalPageURL`, `legalAttributionText`.

`PrecipitationAmountByType` and `SnowfallAmount` are mirrored as models only. Their official pages were not fetched with
docc.py; the names come from the WeatherKit landing page (REA below). Pressure trend, historical statistics, and the
widget location and network row are not touched in this round.

## REA evidence

- WeatherKit landing page (`https://developer.apple.com/documentation/weatherkit`): `ev_fbca4ffbcc8868d71c3ad962cf506acac0c646d1153738d6f1b10487a5ab21d0`
- WeatherAlert page (`https://developer.apple.com/documentation/weatherkit/weatheralert`): `ev_e522863a89e13689d420a6081e1970a8a1ff44bd5b009dbd716ed5e4fe0bf0b1`

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| weatherkit | Precipitation | partial | springboard/ui/src/precipitation.rs |
| weatherkit | Minute forecast | partial | springboard/ui/src/precipitation.rs (MinuteForecast, MinuteWeather) |
| weatherkit | Weather alerts | partial | springboard/ui/src/weather_alert.rs |
| weatherkit | Data attribution | partial | springboard/ui/src/weather_attribution.rs |

Status is `partial`, not `parity`: the models are not yet fed by any data source. The Weather app's Open-Meteo request
still asks for no precipitation amounts, minute data, or alerts, and no SDK exposes these types to apps yet. The
WidgetKit row (location and network access in widgets) is not closed by this round.

## Gaps left open

- No data source: `forecast_url` in `springboard/src/shell/weather/data.rs` does not request precipitation amounts,
  minute data or alerts, so the new types have no producer yet.
- No SDK surface: nothing in `springboard/sdk/src` exposes these models to apps.
- Pressure trend and historical statistics (rows in the same theme) are not built.

## Verification

Per the task rules no build or test was run. `python3 tools/check-names.py` was run and exited 0.
