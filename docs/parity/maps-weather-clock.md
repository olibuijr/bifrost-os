# Maps, Weather, Clock: daily polish (round 75)

Slug: `maps-weather-clock`. Apps: `springboard/apps/maps-desktop` (library crate `bifrost-maps`, no window yet),
`springboard/apps/weather-desktop`, `springboard/apps/clock-desktop`.

## What changed

### Clock: alarms ring and notify with the app closed (1.5.0)

- Alarms are fired by the shell's headless Clock service (`springboard/src/shell/clock/mod.rs`), not by the app, so
  they ring with the Clock closed. The fired alarm's notification path (`notify_alarm` in
  `springboard/src/shell/notify/mod.rs`) looked the app up by its data id `is.olibuijr.clock`, which matches no
  installed manifest (the manifest id is `is.olibuijr.clock.desktop`). Every alarm notification was dropped.
  Fixed: the lookup uses `clock::MANIFEST_ID`, and a unit test checks that the constant matches
  `apps/clock-desktop/manifest.toml`.
- The manifest summary no longer says background alarms are unsupported on PC.
- Already present, not changed: world clocks with the Notification Centre timeline, stopwatch with copy laps, timers
  with the Dock progress bar and bounce, alarm snooze, skip and repeat.
- Midget check needed: set an alarm one minute ahead with the Clock closed, confirm the alert screen and the
  Notification Centre entry under Clock.

### Maps: routing and offline search (model only)

- New `springboard/apps/maps-desktop/src/route.rs`:
  - `Router`: an OSRM-compatible routing base URL. With none configured, `route_url` refuses, so no request is made.
  - `Profile` (driving, walking, cycling) maps to the OSRM profile names.
  - `parse_route`: reads an OSRM `route` reply (distance, duration, steps with manoeuvre and street name), and turns a
    non-`Ok` `code` into an error with the server's message.
  - `estimate`: a great-circle distance with a rough speed per profile, marked `approximate` and with no steps. The UI
    must label it as an estimate.
  - `search_offline`: case-insensitive word match over a local list of `geocode::Place`, for use when the geocoder
    is unavailable.
- Not done: the Maps app has no `manifest.toml`, no `main.rs` and no window, so none of this is reachable from the
  desktop yet. No routing server is configured by default. The current-location model is not wired.
- Midget check needed: none until a window exists.

### Weather: no change this round

- The forecast request already asks for 10 days and 24 hours from now (`springboard/src/shell/weather/data.rs`).
  The hourly strip and the chart draw from that data.
- Alerts: the forecast source (Open-Meteo) sends no alerts, so the footer keeps saying alerts are not available.
  Showing alerts needs an alert source chosen and connected first. Not done.

## Reference evidence

- No macOS 27 reference files were consulted this round (`~/Projects/macos-ref/extract/ref27` was not read). The
  Clock and Maps behaviour above comes from the existing code, the manifests and the OSRM and OpenStreetMap
  geocoder reply shapes already used in the crate.

## Matrix entries closed

- None closed. The lead should not mark any Maps, Weather or Clock row `parity` on this evidence alone: the Clock
  alarm notification needs a Midget run first, and Maps has no window.
