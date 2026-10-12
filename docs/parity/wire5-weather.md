# Wire 5: Weather (1.4.0) wires the attribution footer and the alert availability

Branch `wire5/weather`, app `springboard/apps/weather-desktop/` (crate `bifrost-weather`). Version 1.3.1 to 1.4.0 in
`Cargo.toml` and `manifest.toml`, with a plain-language `CHANGELOG.md` entry at the top. Not built in this round: the
lead runs the one build and the host tests. `springboard/Cargo.lock` is not edited here; the lead's build updates the
`bifrost-weather` version entry.

## What is now wired

| Model (ui/src) | Used for | Where |
|---|---|---|
| `weather_attribution::WeatherAttribution` (`new`, `line`) | The data footer: "Weather data from Open-Meteo" under the last card | `insights.rs` `footer_lines`, `DATA_SERVICE`, `DATA_LEGAL_URL`; `weather.rs` footer loop after the MAP card |
| `weather_alert::WeatherAvailability` (`new`, `has_alerts`) | The footer line that says alerts are not available for the place (the forecast request asks for no alerts) | `insights.rs` `footer_lines`, `DATA_HAS_ALERTS` |
| `weather_alert::most_severe`, `WeatherAlert`, `Severity::description` | The severe weather banner text (`alert_banner`): the most serious alert, its region and threat level | `insights.rs` `alert_banner` (not drawn yet) |

The SDK reaches these as `bifrost_app::controls::{weather_attribution, weather_alert}` (`springboard/sdk/src/lib.rs`
re-exports `bifrost_ui` as `controls`). Both modules are `pub mod` in `springboard/ui/src/lib.rs`.

## How a user reaches it

- **Data footer**: open Weather, pick a place and scroll the forecast to the bottom, below the MAP card. Two lines in
  footnote grey: "Weather alerts are not available for this place from the data service." and "Weather data from
  Open-Meteo".

## Already wired before this round (not changed)

- Hourly strip and 10-day forecast: `springboard/shell/weather/data.rs` (`Hour`, `Day`, `parse_forecast`) and
  `weather.rs` (`HOURLY_TOP`, `DAILY_TOP`). The forecast models in `ui/src` do not exist; the data module is the source.
- Wind unit follows the C/F switch (wire 2, `weather_model::Wind`).

## Behaviour decisions

- **Alerts banner is not drawn.** `insights::alert_banner` returns the text for the most serious alert and `None` for
  an empty list. The app has no alert source: Open-Meteo's forecast request carries no alert data, and no shell service
  feeds one. So the banner has nothing to show and no state field was added for it. Connecting an alert source is the
  next step; then draw the returned text in a card above the hourly strip.
- **Attribution text** uses the model's default line. The legal page URL is held by the model, but the kit has no link
  control in the footer, so the page is not opened from here.
- **Units** are unchanged: `sdk/src/locale.rs` `Measurement` has no speed or pressure unit, and the forecast does not
  request pressure (`pressure_trend` stays unused). Wind follows the C/F switch as before.

## How to verify on Midget (the test PC)

1. After the lead's release: `bifrost pc midget ota` until `phase=accepted`, then
   `bifrost pc midget open-app is.olibuijr.weather.desktop`.
2. `bifrost pc midget swipe` up to the bottom of the forecast; the two footer lines sit under the MAP card. Capture with
   `bifrost pc midget screen /tmp/weather-footer.png`.
3. Light and Dark: `bifrost pc midget appearance light` and `dark`; the footer stays readable on the sky backdrop.
4. Host preview: `bifrost pc smoke weather-desktop --sizes 1200x780 --scale 1.5625` (and `--light`).

Host unit tests (for the lead): `cargo test -p bifrost-weather --offline` in `springboard/`. New tests are in
`insights.rs`: `footer_says_alerts_are_unavailable_then_credits_the_service`, `alert_banner_is_absent_without_alerts`,
`alert_banner_names_the_most_severe_alert_with_its_trimmed_region`, `alert_banner_without_region_omits_the_for_clause`.
The compliance snapshots (`tests/snapshots/`) of the weather screen will differ below the fold; the lead re-blesses.

## REA evidence (official Apple documentation, rendered)

- `ev_a252fd7d81b20ac3d9bf55801c0fdb5a18164d55927227563c5e43c5382caef3`: WeatherAlert page (developer.apple.com/documentation/weatherkit/weatheralert):
  summary, region, severity, source, detailsURL; "not all alerts are severe"; data served raw.

DocC (`python3 -I tools/parity/docc.py`): `weatherkit/weatheralert`, `weatherkit/weatherattribution` (serviceName,
legalPageURL, legalAttributionText for apps that cannot show the attribution page).

## Remaining gaps

- No alert source: the banner is not drawn and the app never has an alert. No alerts from Open-Meteo.
- Attribution: no clickable legal link; the Apple-style combined mark is not used (and is not applicable to another provider).
- No minute forecast, no wind direction or gust, no pressure, no day-part forecasts (see the wire 2 note and matrix).
- No `WeatherError` change in this round.

## Matrix rows touched

Rows are not edited here (the matrix is shared). The lead updates them in the same round as the release.

| Slug | Feature (as in `docs/parity-matrix/data/weatherkit.json`) | BifrOSt path |
|---|---|---|
| weatherkit | Data attribution | `springboard/apps/weather-desktop/src/insights.rs` `footer_lines`; `weather.rs` footer after the MAP card |
| weatherkit | Weather alerts | `springboard/apps/weather-desktop/src/insights.rs` `alert_banner` and `footer_lines` (alert availability); banner not drawn |
