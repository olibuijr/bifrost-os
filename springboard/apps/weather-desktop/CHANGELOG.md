# Changelog

## 1.8.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.8.0 — 2026-10-17

- My Location: picking it asks for location (When In Use, never at launch), then lists the device position first in the sidebar and the Locations list, rounded to two decimals and labelled with its coordinates. Weather's purpose string is in the manifest.
- The precipitation tile names the coming days with rain.

## 1.7.3 — 2026-10-14

- Wind and pressure dial tick spacing uses the shared extra-small spacing token.

## 1.7.2

- Named the dial extent as geometry rather than spacing so the wind and pressure diagrams keep their intended dimensions.

## 1.7.1 — 2026-10-14

- Forecast charts use named scheme colours and scale their day-row layout with the card width; chart marks and spacing follow the appearance and shared spacing tokens.

## 1.7.0 — 2026-10-14

- Air quality card names the main pollutant (PM2.5, PM10, ozone, nitrogen dioxide or sulphur dioxide, whichever is furthest
  over its European Good limit) with its concentration, and the advice for the band.
- Pressure tile draws a trend dial: a needle swings toward rising or falling with the three-hour change.
- Rain tomorrow: a place with notifications on is told once a day when tomorrow's chance of precipitation is 60 percent or more.
- Precipitation tile says how much of today's total is snow.
- A "Current Weather" intent for Search and the assistant answers with the selected place's summary. It is a shortcut
  with no parameters.

## 1.6.0 — 2026-10-13

- Detail tiles: wind direction and gusts with a compass dial, dew point under humidity, a sun path on the sunrise tile, and new Precipitation, Visibility and Pressure tiles that follow the unit setting.
- A Next Hour card with a summary sentence and quarter-hour precipitation bars; precipitation chance under each hour and each day.
- The Locations sidebar shows each place's temperature; Delete removes the selected place.
- The forecast request asks for the extra columns (wind, dew point, pressure, visibility, precipitation, minutely_15).

## 1.6.0 — 2026-10-13

- New detail tiles: Precipitation (the day's total and the next 24 hours), Visibility, Pressure (with a rising, falling
  or steady note) and Averages (today's high against the mean high of this date over the last ten years). Amounts
  follow the unit setting: mm, km and hPa with Celsius; in, mi and inHg with Fahrenheit.
- Air Quality card with the European index, its band and PM2.5, over a scale from good to extremely poor. It comes from
  Open-Meteo's air-quality service and is kept in the cache like the forecast.
- Next Hour card: the minute forecast (15-minute steps) as bars, under a line such as "Rain starting in 30 minutes".
  The footer no longer lists the minute forecast as missing; weather alerts are still not served by the data source.
- Wind tile: gusts, the bearing the wind comes from and a compass dial. Humidity tile: the dew point. Sunrise tile: the
  sun's day arc with the sun at its place.
- Hourly forecast: the chance of precipitation under each hour, and a column for the sunrise or sunset where it falls.
  10-day forecast: the chance of precipitation under each day's glyph.
- Locations sidebar: each place shows its sky glyph, the sky in words and its temperature. Places can be dragged to
  reorder them (the order is saved), removed with the Delete key and removed or switched with a secondary click.
- Rain notifications: a Notify switch per place on the Locations page. When a refresh shows rain starting within the
  hour at such a place, the app posts one notification for that spell of rain. New permission: notifications.
- Share button in the toolbar sends the place, temperature, sky and range as text to the share menu.
- `weather://` links: `weather://Akureyri` shows a saved place or looks the name up and adds the first match;
  `weather://?lat=65.68&lon=-18.1&name=Akureyri` adds that position directly.
- Control methods `weather.places`, `weather.current` and `weather.forecast` return what the window shows as JSON, with
  an optional `place` parameter naming a saved place.

## 1.5.3 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.5.2 — 2026-10-12

- Removed `insights::alert_banner`, which nothing drew. The forecast request asks for no alerts (`DATA_HAS_ALERTS` is
  false) and the data model has no alert field, so the footer's "Weather alerts are not available" line is what the
  screen shows. A banner comes back with the alert data, from the shared `weather_alert` control.

## 1.5.1 — 2026-10-12

- Removed the unused needs_frame helper (tick already reports when a frame is needed); the state dump is test-only.

## 1.5.0 — 2026-10-12

- The temperature chart has an area fill under its line, so the warm and cold hours of the day read at a glance.

## 1.4.0 — 2026-10-12

- The bottom of the forecast says where the data comes from: "Weather data from Open-Meteo", so the source is credited on
  the screen that shows its numbers.
- The same footer says plainly when weather alerts are not available for a place. The forecast service sends no
  alerts, so the app says so instead of implying that there are none.
- The app has the code for a severe weather banner, which names the most serious alert, its region and its threat
  level. It is not shown yet, because no alert source is connected.

## 1.3.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.3.0

- The forecast has a Temperature chart: the hours as a line, with labelled temperatures on the side and the times
  along the bottom. The chart is in the same card style as the rest of the forecast.
- The forecast has a Map card for the selected place: a grid of latitude and longitude lines with the place at the
  centre and a scale bar in km or miles to match the temperature setting. The card says plainly that no map tiles
  are loaded, so the grid is the only map.
- Locations ends with a note about location access. Weather uses the places you add and does not read this PC's
  position; the note says so in plain words, and a failed location request would be explained the same way.

## 1.2.3

- A Moon tile in the forecast shows the moon's lit share and its phase, such as "Waxing Gibbous", for right now.
- An error screen names the problem in plain words and says what to do next: allow weather access, or check the
  connection and try again.
- Wind speed follows the temperature setting: km/h with Celsius, mph with Fahrenheit.
- Each other place in Locations shows how far away it is from the selected place, in km or miles to match the setting.
- The current conditions for the selected place are offered to Notification Centre as a widget. The entry shows the place,
  the temperature and the sky, and it is refreshed about every 15 minutes or when the forecast changes.
- Weather follows the system's network path: with no connection and nothing saved, the error screen shows; when the
  connection comes back, the forecast is fetched at once instead of waiting for the next retry.

## 1.2.2 — 2026-10-11

- Background forecast and quote fetches run on the shared dispatch queue; finished results apply on the app loop, so
  nothing polls every 100 ms and no worker pool is kept.
- Every visible control has an accessibility label and a press action: the Celsius and Fahrenheit buttons, the search
  and symbol fields, the sidebar and Locations rows, and the add and remove buttons.
- Grouped cards use the system 13 pt radius; the falling-stock colour is the system red, not the destructive colour.
- Section headers in Locations are secondary grey; error notes no longer draw in red.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate window: full-size, with a Locations sidebar (search capsule and the saved places as a source list, the
  traffic lights in its top band) beside the forecast. A narrow window hides the sidebar and shows a Locations button.
- The navigation row is a unified toolbar: Stocks on the forecast, Done on Locations, Back on Stocks.
- Locations and Stocks follow Light and Dark (system colour pairs); the forecast keeps white text over the sky.
- Toolbar buttons and the sidebar search are published as controls. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 23 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Weather for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
