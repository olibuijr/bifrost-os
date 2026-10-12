# Changelog

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
