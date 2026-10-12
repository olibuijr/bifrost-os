# Connect: Clock app to the system services

Branch `connect/clock`, from `parity/integration`. App `springboard/apps/clock-desktop/` (`bifrost-clock` 1.2.3, id
`is.olibuijr.clock.desktop`). No builds were run this round: the lead builds once. Run `cargo test` in
`springboard/apps/clock-desktop` and check the Midget steps below before merging.

The app's previous-round wire note (`wire-app-clock.md`) does not exist; the app had no service calls before this round.

## What is connected

| Service | Matrix row (`docs/parity-matrix/data/`) | What the app does | Status |
|---|---|---|---|
| `widgets_timeline` (World Clock widget) | `widgetkit.json`, "Timeline updates" | Writes the chosen cities as a timeline of one entry per minute for the next hour (`reload`, kind `world-clock`, policy `at_end`). Notification Centre shows the entry on screen. | Connected. Unverified on Midget. |
| `dock` (progress and bounce) | `appkit-user-interface.json`, NSDockTile and `requestUserAttention` | An indeterminate progress bar on the Dock icon while a timer runs; the bar clears when no timer runs; one bounce when a timer finishes. | Connected. Unverified on Midget. |
| `notification-schedule` | `user-notifications.json`, "Notification triggers" | Not connected. See "Why alarms are not scheduled again" below. | Not connected. |

Code:

- `springboard/apps/clock-desktop/src/system.rs`: the glue. `world_entries` and `line` build the entries, `System`
  keeps what was last sent and sends only on change (`sync_world`), and `timers` drives the Dock bar and bounce.
- `springboard/apps/clock-desktop/src/main.rs`: `ClockApp` calls the glue. `configure` sends the timeline once at startup;
  `event` (touches and keys) sends it when the cities changed and updates the Dock bar; `tick` detects a timer that
  finished (the running count dropped) and bounces the Dock icon; `service_event` answers the shell's `reload`
  request; `service_reply` logs an error reply and otherwise ignores it.
- `springboard/apps/clock-desktop/Cargo.toml`: adds `bifrost-core` (the `Json` type of the `App` service hooks).
  Other apps already depend on it the same way.

Calls are made only on changes: a changed city list, a timer that starts, stops or finishes, or the shell's reload
request. Nothing is sent every frame.

## How a user sees it

- **World Clock widget.** Open Notification Centre (swipe down from the left two thirds of the status bar). The widget
  area shows "World Clock" with the first four chosen cities and their local times, for example
  "Reykjavik 12:00, London 13:00, New York 08:00, Tokyo 21:00". The line changes every minute while the app runs.
  Adding, moving or removing a city in the app updates it at once.
- **Dock progress.** Start a timer in the app's Timer tab. The Dock icon shows an indeterminate bar along its lower part
  until the timer stops or finishes.
- **Dock bounce.** When a timer finishes, the Dock icon bounces once. The shell's full-screen alert still shows as before.

## How to verify on the test PC (Midget)

Use Midget only. Update the shell and the app first (`bifrost pc publish`, `bifrost pc midget ota` until
`phase=accepted`); the Clock is an installed app, so reinstall it with `ssh midget` then
`/usr/bin/springboard --install-store-app is.olibuijr.clock.desktop` if the store copy is older.

1. Open Clock (World Clock tab). Open Notification Centre and check the "World Clock" widget shows the default cities
   with times that match the clock faces.
2. Tap the edit button, remove a city, and open Notification Centre again. The widget line loses that city at once.
   Add a city back with the add button and check it appears in the widget.
3. Start a 1 min timer in the Timers tab. Take a screenshot (`bifrost pc midget screen PATH`). Expect a progress bar on
   the Clock's Dock icon. Stop the timer; the bar goes away.
4. Start the same timer again, let it run out, and check the Dock icon bounces once and the alert shows.
5. Read the shell log (`bifrost pc log`) and the app's stderr for `clock: service request ... failed`. A shell without
   these services answers with an error; the line is then expected and the app must otherwise work normally.
6. Keep the app running for an hour. Expect a new timeline to be sent when the hour runs out (the `reload` event), and
   the widget to keep its times.

## Why alarms are not scheduled again through `notification-schedule`

The shell already turns alarms and timers into notifications. `springboard/src/shell/mod.rs` calls
`Shell::notify_alarm` (`springboard/src/shell/notify/mod.rs`), which posts a notification for each fired alarm or timer
(`Your timer is done.` or the alarm time). Adding a second path through `notification-schedule` would show every alarm
twice when the Clock is open.

Gap found while reading it, not verified on a device: `notify_alarm` looks the Clock up with
`apphost::slot_of("is.olibuijr.clock")` and drops the notification (`continue`) when that package id is not installed.
The Clock app that ships is `is.olibuijr.clock.desktop`, and no manifest with id `is.olibuijr.clock` exists in the repo.
If no such package is installed, alarms and timers fire the full-screen alert but no banner or Notification Centre entry.
The fix belongs in the shell (look the Clock up by a name the shell owns, or post without an app slot) and is not made
here. Once that is settled, the app could schedule a fallback `notification-schedule` request with the same identifier
per alarm, which replaces a pending one; that needs the `notifications` permission in the manifest, which this round does
not add.

## REA evidence

No new official page was relied on this round. The Dock and timeline behaviour follows the existing wire notes:
`docs/parity/wire-svc-dock.md` (NSDockTile, `requestUserAttention`) and `docs/parity/wire-svc-widgets.md` (WidgetKit
timelines, `reload` and `at End` policy).

## Gaps left

- **Notifications for alarms and timers:** the shell's `slot_of` attribution gap above (shell side).
- **Timeline and widget when the app is closed:** the timeline stays in the shell's memory, so the widget keeps the last
  hour it was given. After that hour it shows the last entry, because no app runs to send a new timeline. It is lost on a
  shell restart; the app sends it again the next time it starts.
- **Widget text:** one line of at most four cities. There is no per-family layout and no tap action from the widget.
- **Dock:** the bar is indeterminate only. The determinate fraction of a timer would need a call every second, which the
  rule against per-frame calls forbids; the Dock shows that a timer runs, not how far it has got.
- **Dock badge:** not used. Apple's Clock shows no Dock badge for timers.
- **Replies:** errors are logged to stderr; the app does not retry a failed call.
- **Tests:** unit tests in `system.rs` check the entries, the change-only sending, the reload request, the Dock bar and
  the bounce, using `Ctx::testing` and the queued `ServiceCall` messages. Not run in this round.
