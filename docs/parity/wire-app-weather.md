# Wire: Weather app (weather-desktop 1.2.3)

Branch `wire/app-weather`, from `parity/integration`. Wires three pure models into the Weather app. No new crates,
no shell changes, no matrix edits.

## What is now wired

| Matrix slug | Feature (docs/parity-matrix/data/weatherkit.json) | Model | Where in the app |
|---|---|---|---|
| weatherkit | Weather errors (`WeatherError`) | `bifrost_ui::weather_model::WeatherError` (`from_io`, `description`, `recovery_suggestion`) | `src/weather.rs` `maintain` (fetch result), `draw_main_with` (error screen), `describe` |
| weatherkit | Current conditions, moon phase (`MoonPhase`, `illuminated_fraction`) | `weather_model::{MoonPhase, illuminated_fraction}` | `src/weather.rs` `moon_text`, Moon tile in the detail grid |
| weatherkit | Current conditions, wind (`Wind::speed_mph`) | `weather_model::Wind` | `src/weather.rs` `wind_text` (WIND tile) |
| weatherkit | Weather service (place distance) | `geo::{Coordinate, format_distance, DistanceUnits}` | `src/weather.rs` `away_text`, `subtitle_with_distance` (Locations rows) |

The SDK reaches these as `bifrost_app::controls::{weather_model, geo}` (`springboard/sdk/src/lib.rs` re-exports
`bifrost_ui` as `controls`).

## How a user reaches it

1. Open Weather (Dock or Spotlight). The forecast for the selected place shows.
2. **Moon**: the sixth detail tile (bottom right, after Feels Like) shows the lit share of the Moon (for example
   `67%`) with the phase name underneath (`Waxing Gibbous`).
3. **Wind**: the WIND tile reads km/h with the Celsius setting and mph with Fahrenheit. The C/F switch in the
   toolbar changes it.
4. **Distance**: in Locations, every place other than the selected one shows its region followed by its distance,
   for example `Iceland · 343 km away`. Fahrenheit shows miles and feet (`mi away`, `ft away`). Search results show
   the same note.
5. **Errors**: with no network and no cached forecast, the screen shows the error description and, under it, what
   to do. A refused read says "Weather data is not allowed for this app." and "Allow weather access in Privacy
   settings and try again."; any other failure says "The weather could not be loaded." and "Check the connection
   and try again." (Previously: "Can't reach Open-Meteo".)

## How to verify on the test PC (Midget)

Build and publish follow `docs/bifrost-pc-workflow.md`; the Midget steps are the usual ones (`bifrost pc midget ota`
until `phase=accepted`, then the screenshot). Then:

- Open Weather with the network up. The Moon tile is the sixth tile; its percentage and phase should match a
  moon-phase calendar for today. `bifrost pc midget screen /tmp/weather.png` to capture.
- Tap the C/F switch to Fahrenheit: WIND shows `mph`; Locations rows show `mi away`.
- Add a second place in Locations: its row shows `away` from the selected place; the selected place shows no distance.
- Disconnect Wi-Fi with no cached forecast (or block the app's network permission): the error screen shows the two
  sentences above. With the permission refused, the first sentence is the permission text.
- `bifrost pc midget state` prints `weather_error=` with the description, empty when a forecast is shown.

Host checks (run by the lead): `cargo test -p bifrost-weather` (the new tests are in `src/weather.rs`, module
`wire_tests`), and `cargo test -p bifrost-ui weather_model` for the model's own tests.

## REA evidence

- `ev_199a2b1fb9be1fa02315a088a6a31becd8a715018b8c85ddaad263486f5407a4`: rendered WeatherError page
  (developer.apple.com/documentation/weatherkit/weathererror): `permissionDenied`, `unknown`, `errorDescription`,
  `recoverySuggestion`.
- MoonPhase: official DocC page read with `tools/parity/docc.py weatherkit/moonphase` (eight phases, `description`).
  No REA capture yet.

## Decisions and gaps

- **Moon** is computed from the mean lunation (`weather_model.rs`): it can be a day off near a quarter. The app
  reads the system clock, so the tile changes by itself; the model does not.
- **Wind direction and gust** are not wired. The forecast the app gets (`Current`, `springboard/src/shell/weather/data.rs`)
  has no direction or gust field. `Wind::compass` and `gust_factor` stay unused until the data module parses them.
- **Units**: the wind unit follows the C/F switch, which is an assumption: the app has no separate speed setting.
  Recheck if a speed unit preference is added.
- **Distance** is great-circle from the selected place; the Locations note does not update when the selected place
  changes until the list is redrawn (it is recomputed each frame, so this is immediate).
- **Measurement** (`sdk/src/locale.rs` `measurement`) is not used: its `Unit` has no speed unit, and the app's
  temperature text keeps `Units::degrees`.
- **Snapshots**: `tests/compliance.rs` pixel snapshots in `tests/snapshots/` (weather-desktop-dark.png and
  -light.png) will differ: the Moon tile is drawn on every loaded forecast and its value changes over time, so the
  snapshot is time-dependent. The lead should re-bless (`bifrost pc acceptance --bless`) after the build and decide
  whether the compliance check should freeze the clock.
- Not built here (no builds in this round), so the new code is unverified until the lead's build.
