# Wire: Calendar app (bifrost-calendar 1.8.4)

Branch `wire/app-calendar`. Scope: `springboard/apps/calendar-desktop/` only, plus this note. The matrix data
(`docs/parity-matrix/`) is not edited; the lead updates the rows after the build.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status |
|---|---|---|---|
| `eventkit` | Change notifications (`EKEventStoreChanged`) | `springboard/ui/src/calendar_store_changes.rs` (`StoreChangeHub`), used by `apps/calendar-desktop/src/calendar.rs` (`changes` field), `src/calendar/exchange.rs` (import) and `src/calendar/alerts.rs` (`step` takes the hub generation) | wired |

Behaviour change: every save, delete or import posts one store change through `StoreChangeHub::post`. The alert
state rescans the alert calendar when the generation moves, so an alert added or changed by a save is scheduled at
once. Several saves between two alert steps cost one rescan.

## What was already in the app (verified in code, not changed)

- Repeats: `src/calendar/recur.rs` is an RFC 5545 RRULE subset (DAILY, WEEKLY, MONTHLY, YEARLY; INTERVAL, COUNT,
  UNTIL, BYDAY with ordinals, BYMONTHDAY, BYMONTH) with `EXDATE` exceptions in `expand`. The store offers the
  repeat choices in `alerts.rs` (`REPEAT_RULES`). Matrix `eventkit` "Recurrence rules" is `partial`.
- Alerts: `src/calendar/alerts.rs` computes due alarms (`due`, `step`) and `src/main.rs` posts each one through
  `ctx.notify`. The alarm choices are `ALERT_MINUTES` (at the start, 5, 15, 30 minutes, 1 hour, 1 day before).
- Time zones: the app reads zones embedded in the .ics data (`IcsCalendar::timezones`) and uses the shell's `Wall`
  offset for floating events. There is no zone picker.

## How a user reaches it

Calendar (Dock or Launchpad) > New Event (Cmd+N) > set a repeat and an alert > Save. Edit an event to move its
alert: the change takes effect without a restart. The Event menu has Save, Duplicate, Export and Delete; the Go menu
has the views.

## How to verify on the test PC (Midget)

Host tests first (run by the lead; no builds were run by this wire round):

    cargo test --offline -p bifrost-calendar --test store_changes
    cargo test --offline -p bifrost-calendar alerts

Midget:

1. `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.calendar.desktop`.
2. Create an event starting about 3 minutes from now with the alert "At time of event", and save. The notification should appear at the start time.
3. Edit the same event: move its start 2 minutes ahead and set the alert to "5 minutes before". Save. The new alert
   should fire about 3 minutes from now, not at the old time (this exercises the store-change rescan).
4. Screenshot with `bifrost pc midget screen /tmp/calendar-wire.png` and check against the Golden Gate layout
   (`bifrost-hig`, rules GG-01 to GG-05).

## REA evidence (official documentation, rendered page)

- `ev_fc582b1411a1a433fe34fcf032d0c5306ac94aef3d37a1f73fe0b0cd73cf3722`: EKRecurrenceRule (eventkit/ekrecurrencerule).
- `ev_01cac1a811760eb350c96c27ff23086528bf60fed760fcd38658fb8aefbe1df8`: UNCalendarNotificationTrigger
  (usernotifications/uncalendarnotificationtrigger).
- DocC reads with `python3 -I tools/parity/docc.py`: `eventkit/ekrecurrencerule`, `usernotifications/uncalendarnotificationtrigger`.

## Remaining gaps

- Calendar notification triggers (`UNCalendarNotificationTrigger`): alerts are computed in the app and posted through
  `ctx.notify`; the SDK `usernotif` module has only the time-interval trigger. Moving alerts to the SDK needs a
  local time-zone clock in the shell (see `docs/parity/user-notifications.md`).
- Time zone database: no time zone model exists in `springboard/ui` or `springboard/sdk`. The shell's tzdata reader
  is in `springboard/src/bare/timezone.rs`, outside this app. The app cannot pick a zone for an event yet.
- Single-occurrence edits: `EXDATE` is read and expanded, but the editor has no "this event only" or "all future
  events" span (`EKSpan`), so an occurrence cannot be removed on its own from the UI.
- Event store access (`calendar_access.rs`) and error codes (`calendar_error.rs`) are not used: the app's store is a
  local file, and its save-failure copy is user-facing text rather than EventKit error codes.
