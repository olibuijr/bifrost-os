# Connect: Calendar app to the shell services (bifrost-calendar 1.8.4)

Branch `connect/calendar`, from `parity/integration`. Scope: `springboard/apps/calendar-desktop/` only, plus this note.
No build was run for this round (the lead builds once). The lead updates the matrix rows after the build.

## What is connected

| Service | Method(s) | Calendar feature | Code |
|---|---|---|---|
| `notification-schedule` | `add`, `remove` | Event alerts. Each alarm of an event is one request on a calendar trigger (weekday, hour, minute, not repeating), with the event title and the alert text. An edit replaces the request under the same identifier; a delete removes it. | `src/calendar/connect.rs` (`plan_alerts`, `diff`, `Connect::sync`) |
| `widgets_timeline` | `reload` (kind `up-next`) | Up Next widget: today's timed events that have not started, one entry per event, shown from now and moved on as each starts. Reloads on store changes, at midnight, and when the shell asks (`reload` event). | `connect.rs` (`up_next_entries`, `send_up_next`, `refresh_up_next`); `main.rs` (`service_event`) |
| `dock` | `badge` | The Dock tile badge shows today's day of the month, updated when the day changes. | `connect.rs` (`badge_text`, `send_badge`) |

Calls happen only on real changes: a save, delete or import (the store-change generation moves), or the local day
rolling over. Nothing is sent every frame. A sync with no change sends nothing.

Identifiers: `calendar-alert:<event uid>:<minutes before>`. One request per alarm of an event, so an event with two
alarms has two requests, and each series keeps one request per alarm for its next occurrence in the window.

## Fallback without the service

An older shell answers `no service notification-schedule`. `Connect::on_reply` records that, and then the app posts its
alert notifications itself from `take_alerts`, as in 1.8.4. Replies for other services do not turn it on.

## How a user sees it

- Create an event with an alert (New Event, set Alert, Save). The notification arrives at the alert time, from the
  notification schedule, even if the Calendar window is closed.
- Edit the event's time or alert: the old alert is replaced. Delete the event: its alert is removed.
- Notification Centre: the Up Next widget lists the next timed events of today.
- Dock: the Calendar tile shows today's day of the month as its badge.

## How to verify on the test PC (Midget)

Host tests (run by the lead): `cargo test --offline -p bifrost-calendar calendar::connect` (the glue tests in
`connect.rs`, which use `Ctx::testing` and `take_sent`), and `cargo test --offline -p bifrost-calendar store_changes`.

Midget:
1. `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.calendar.desktop`.
2. Create an event about 3 minutes ahead with the alert "5 minutes before"... (use "At time of event" so it fires in
   about 3 minutes), and save. The notification should appear at the start time. Close the window first to prove the
   schedule works without the app open.
3. Edit the event to start 2 minutes later. The alert should now fire at the new time, not the old one.
4. Delete the event. No notification should arrive at its old time.
5. Open Notification Centre: the Up Next widget lists the next timed event of today (`bifrost pc midget screen PATH`).
6. Dock: the Calendar tile shows today's date number as its badge. Read it with `bifrost pc control call` on the dock
   service state, or from the screenshot.

## Gaps

- Trigger dates: the shell's calendar trigger takes a weekday and a time, not a date, so the schedule holds only the
  next occurrence within six days and 22 hours. Alerts further out are scheduled when the day rolls over or the store
  changes, so an alert is missed only if the app is closed through that whole window and the alert day is reached.
- Repeating events are scheduled one occurrence at a time (non-repeating triggers), not with a repeating calendar
  trigger.
- Time zones: Up Next uses the device's zone offset for every event; alerts use each event's zone through the existing
  `due` logic. The Up Next list does not model events in other zones.
- Dock icon: the badge is the day number; the icon itself does not show the date (no dock icon drawing service exists).
- `Cargo.lock` gains the `bifrost-core` dependency of `bifrost-calendar`; the lead's offline build updates it.
- `docs/bifrost-ui.md` "System services" does not list these uses; the lead adds the matrix rows after the build.
