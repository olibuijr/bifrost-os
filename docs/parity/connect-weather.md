# Connect: Weather app to the system services (branch connect/weather)

The Weather app (`springboard/apps/weather-desktop`, version 1.2.3, the top CHANGELOG entry) now calls two shell
services: `widgets_timeline` (the WidgetKit-style timeline behind Notification Centre) and `network` (the network
path). Before this change no shipped app called either service (see `wire-svc-widgets.md` and `wire-svc-network.md`,
"Nothing calls the service in a shipped app"). Weather is the first.

No shell, SDK or other app code changed. The app adds one dependency (`bifrost-core`, for the JSON type named in the
service hooks) to its own `Cargo.toml`.

## What is connected

| Service | Matrix slug (`docs/parity-matrix/data/`) | Feature | Where in the app |
|---|---|---|---|
| `widgets_timeline` | `widgetkit` | Timeline updates: `TimelineEntry`, `Timeline`, reload policy `after(_:)`, `WidgetCenter` reload | `src/services.rs` (`entries`, `push_timeline`), `src/main.rs` (`push_timeline`, `sync`, `service_event`) |
| `network` | `network` | Network path monitoring: `NWPath.status` (satisfied or not), retry when the path returns | `src/services.rs` (`online`), `src/main.rs` (`configure`, `service_reply`, `service_event`), `src/weather.rs` (`set_online`, `maintain`) |

### Widget timeline (current conditions)

- The app keeps one timeline kind, `current-conditions`. Its single entry starts now, its title is the selected place,
  and its text is the temperature in the app's units followed by the sky (for example `4° Cloudy`).
- The reload policy is `after(now + 15 min)`: the shell asks the app for a new timeline at that date, and the app
  answers with the current summary and a new 15-minute date. The 15 minutes match the forecast refresh interval.
- The timeline is sent when a real change happens: the first start (once), a forecast arrives for the selected place,
  the selected place changes, or the temperature unit changes. Nothing is sent on a timer or every frame. The app
  sees these changes through `Weather::take_summary_changed`, which is set where the saved summary is written.
- With no forecast known for the selected place, the app removes its timeline (`remove`) instead of sending an empty one.
- A reload event from the shell (`reload` with the kind) makes the app send the current timeline again.
- An older shell answers `no service widgets_timeline`. The app records that once and sends nothing more to the service.

### Network path (offline state and retry)

- At start the app asks `network.path` once. The reply, and every later `pathUpdate` event, tells the app whether
  the path is satisfied. Only `satisfied` counts as online; `requiresConnection` (an address still pending) counts as
  offline until it is satisfied.
- Going offline stops the forecast fetches. If no forecast is cached for the selected place, the Weather screen shows
  the error screen with the existing sentences: "The weather could not be loaded." and "Check the connection and try
  again." Cached forecasts keep showing.
- Coming back online clears the retry wait for every place and fetches at once, instead of waiting for the 30-second
  retry or the 15-minute refresh.
- A failed path request (an older shell, or `no service network`) leaves the app online, which is how it behaved before.
- Repeated identical states change nothing (`Weather::set_online` returns early).

## How a user sees it

1. **Notification Centre widget.** Open Weather, choose a place with a forecast. Open Notification Centre (swipe down
   from the left two thirds of the status bar, or tap the clock). The widget area shows a row titled with the place,
   with the temperature and sky as its text. Changing the selected place in Locations or the C/F switch updates the row.
2. **Offline.** Disconnect the network with no forecast cached for the place (or use a place the app has not fetched
   yet). Weather shows the error screen with the two sentences above.
3. **Back online.** Reconnect. The forecast loads without waiting for the retry delay; the error screen gives way to
   the forecast.

## How to verify on the test PC (Midget)

Midget only (the VM is not used). Build and publish as in `docs/bifrost-pc-workflow.md`, then `bifrost pc midget ota`
until `phase=accepted`.

1. Open Weather with the network up and let a forecast load. Open Notification Centre: the widget row shows the
   selected place, the temperature and the sky. `bifrost pc midget screen /tmp/weather-widget.png` to capture.
2. Tap the C/F switch: the widget row's temperature changes to Fahrenheit text within a moment.
3. Unplug the Ethernet cable (Midget uses wired VLAN 20) with the app on a place that has no cached forecast (for example
   a freshly added place). Expect the error screen; `bifrost pc midget state` prints `weather_error=The weather could not be loaded.`.
4. Plug the cable back in. Expect the forecast to appear within a few seconds, without waiting 30 seconds.
5. With Weather closed and reopened, the widget row returns from the saved summary (the app re-sends its timeline at start).
6. Host checks (the lead's build): `cargo test -p bifrost-weather services` for the glue, and
   `cargo test -p bifrost-weather going_offline` for the offline state in `src/weather.rs`.

## Gaps

- **No offline sentence.** `bifrost_ui::weather_model::WeatherError` has only `PermissionDenied` and `Unknown`. Offline
  uses `Unknown`, so the screen says "Check the connection and try again", not "You are offline". Adding an offline
  variant needs a change in `springboard/ui` (not owned by this round).
- **Cached forecasts show no offline note.** The screen keeps the last forecast while offline and does not mark it stale.
- **Timeline shows one entry.** No hourly or daily entries, no relevance score, no widget families, no interactivity.
  The model supports more entries; the app keeps to current conditions.
- **Timelines are in memory on the shell side.** They are lost when the shell restarts. The app re-sends its timeline when
  it starts or when a reload event arrives; it does not know a restart happened until the next change.
- **Only the selected place is in the widget.** The timeline kind is one per app.
- **Path details unused.** Interfaces, expensive and IPv4/IPv6 are decoded by the SDK but not shown. Low Data Mode
  (`constrained`) is not in the model, so no path can be expensive-aware here.
- **Reachability of the forecast host is not probed.** A satisfied path means a route exists, not that Open-Meteo answers;
  a failed fetch still shows the error screen with the same sentences.
- **Cargo.lock.** The new `bifrost-core` dependency needs `springboard/Cargo.lock` to list it under `bifrost-weather`
  (the lock is already one version behind the manifest). The lead's build updates it unless it runs with `--locked`.
- **Not compiled.** Per the round's rules, nothing here was built or run. The lead's single build is the first check.
- **Matrix rows not edited.** The `widgetkit` and `network` rows stay as the wire notes left them; their `note` can
  cite Weather once the build is confirmed on Midget.
