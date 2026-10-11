# WeatherKit parity: wave 2

Branch `parity/weatherkit`. Adds a pure weather model to bifrost-ui, with no network and no new crates.

## What was built

`springboard/ui/src/weather_model.rs` (declared in `springboard/ui/src/lib.rs`):

- `MoonPhase` (eight cases, `label`, `is_waxing`, `from_age_days`, `at_unix`). The phase comes from the mean
  lunation with a fixed reference new moon, so it can be a day off near a quarter. This is a model, not an ephemeris.
- `illuminated_fraction` and `age_days`, computed from Unix seconds (UTC). The caller passes the time in.
- `CompassDirection` (eight points: cardinal and intercardinal) and `Wind` (`speed_kmh`, `direction_deg` folded
  into 0..360, optional `gust_kmh`, `compass`, `speed_mph`, `gust_factor`).
- `WeatherError` (`PermissionDenied`, `Unknown`), with `from_io`, `description`, `recovery_suggestion`, `Display`
  and `std::error::Error`.

Tests: 8 `#[test]` functions in the module, covering the new moon, full moon, quarter bins, wrap-around of ages,
the eight bins in order, compass edges at 22.5 degrees, wind folding and gust factor, and the error mapping.

Not built in this wave, and still open:

- `springboard/src/shell/weather/data.rs` does not request wind direction or gust from the forecast endpoint, so
  the Weather app still shows speed only. Wiring `Wind` into `Current` needs a build to check the Weather app tests.
- No moonrise, moonset, dawn or dusk. `MoonEvents` and `SunEvents` need an ephemeris and are not built.
- The sixteen-point `CompassDirection` refinement is not verified: the official page for the compass enum was not
  readable through the DocC tool (404 on `weatherkit/wind/compassdirection`). Only the eight-point model is built.
- No SDK type exposes weather to other apps. The model is in bifrost-ui, not `springboard/sdk/src/`.

## Official API names mirrored

- `MoonPhase`: `.new`, `.waxingCrescent`, `.firstQuarter`, `.waxingGibbous`, `.full`, `.waningGibbous`,
  `.lastQuarter`, `.waningCrescent`
- `Wind`: `speed`, `direction`, `gust`, `compassDirection`, and `Wind.CompassDirection`
- `WeatherError`: `.permissionDenied`, `.unknown`, `errorDescription`, `recoverySuggestion`

Source pages (DocC, read with `tools/parity/docc.py`):

- https://developer.apple.com/documentation/weatherkit/moonphase
- https://developer.apple.com/documentation/weatherkit/wind
- https://developer.apple.com/documentation/weatherkit/weathererror

## REA evidence

| Page | evidence_id |
|---|---|
| WeatherKit `MoonPhase` | `ev_fdb05993c9cce84676264531a96c6205e500c779d88d0cffae59bfffdea52248` |
| WeatherKit `Wind` | `ev_37439e073a2677c502237d800a81e059bc54aac27a34715edd63be822b9a4e51` |
| WeatherKit `WeatherError` | `ev_b8ec31671ba6996902eb92394b530796460a52d1e53ff8fdac06bed3db62d0b6` |

Landing page evidence from wave 1 (WeatherKit collection): `ev_640a3b252271e036de45d2e26c328a09c623b3a03e40a778617a658c969c3ffc`.

## Matrix rows closed

These rows are for the lead to apply to `docs/parity-matrix/data/weatherkit.json`. This branch does not edit that file.

| framework | feature (as written in data/weatherkit.json) | new status | BifrOSt path |
|---|---|---|---|
| WeatherKit | Wind | partial | springboard/ui/src/weather_model.rs (`Wind`, `CompassDirection`) |
| WeatherKit | Weather errors | partial | springboard/ui/src/weather_model.rs (`WeatherError`) |
| WeatherKit | Sun and moon events | partial | springboard/ui/src/weather_model.rs (`MoonPhase`, `illuminated_fraction`) |

The `Sun and moon events` row stays partial: it adds moon phase only, and sunrise and sunset were already covered
in `springboard/src/shell/weather/data.rs`.

## Verification

Builds were not run, per the task rules. Checks done:

- `python3 tools/check-names.py` exits 0 (no Apple names in the code).
- Braces and parentheses balance in `weather_model.rs`; 8 test functions are present.
