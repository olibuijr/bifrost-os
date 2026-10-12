# Wiring: Reminders app (1.10.0)

The Reminders app (`springboard/apps/reminders-desktop/`) now uses the shell's notification triggers for due-date
alerts, instead of posting a notification only while the app is running. The app's own lists, priorities and
repeat rules were already in place and are unchanged.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/user-notifications.json`) | BifrOSt path |
|---|---|---|
| user-notifications | Notification triggers | `springboard/apps/reminders-desktop/src/alerts.rs` (planner and `apply`), on the SDK `usernotif::add` with `Request::after_ms` (`springboard/sdk/src/usernotif.rs`) and `remove_pending` |

What changed in behaviour:

- Each upcoming due alert and early alert is a pending request with a time-interval trigger
  (`UNTimeIntervalNotificationTrigger`, `after_ms`). The shell fires it on time, so the alert shows even when the
  Reminders window is closed.
- Request identifiers are `reminder-<id>-due` and `reminder-<id>-early`. A changed reminder replaces its request; a
  completed or deleted one has its request removed (`remove_pending` with its identifiers).
- The app keeps at most 32 alerts pending (the shell allows 64 per app) and only those within the trigger limit of
  about 49 days. The soonest 32 are planned.
- The app re-plans only when the saved data changes (generation), when the time zone offset changes, when the
  earliest planned alert is due, and at least once an hour (so the window moves forward). A plan that did not change
  sends nothing, and an idle app does not read the store.
- On the first sync after the app starts, every pending request of the app is removed (`remove_pending` with no
  identifiers) and the current plan is added. This clears alerts for reminders deleted while the app was not
  running.
- Completed reminders have no alert. A repeating reminder's next alert is scheduled when it is completed, because
  completion saves the data and triggers a re-plan.
- Alerts that were already due at the first sync are not fired. This matches the earlier behaviour.

Unchanged: the dock badge (due count), the Reminder menu, the App Intent, lists, priorities, subtasks, repeat rules
(`src/repeat.rs`) and the store file format.

## How a user reaches it

1. Open Reminders, open a reminder's editor, and set its due date, its time (Time row) and optionally the Early row
   (an alert 5 minutes to 2 days before).
2. Close the Reminders window. The alert still appears at the due time as a notification banner, and it appears in
   Notification Center.
3. Changing or completing the reminder replaces or removes the alert.

The notification permission comes from the manifest (`permissions = ["storage", "notifications"]`), as before.

## How to verify on the test PC (Midget)

The owner rule is to test on Midget only (no VM).

1. Build and publish as usual: bump is already done (1.10.0), so run `bifrost pc publish` and then
   `bifrost pc midget ota` until `phase=accepted`. Check the version with `bifrost pc midget state`.
2. Open Reminders with `bifrost pc midget open-app is.olibuijr.reminders.desktop`, add a reminder titled "Wire test"
   due in about two minutes with an early alert of 1 minute, then close the Reminders window.
3. Wait. Expect a banner "Wire test", body "Due in 1 minute" at the early time and "Due <time>" at the due time, with
   the Reminders window closed.
4. Add a second reminder, complete it before its time, and confirm no banner appears for it.
5. Capture the result with `bifrost pc midget screen /tmp/reminders-alert.png` as evidence.

Host tests (not run in this round, per the wave rule): `cargo test -p bifrost-reminders` covers the planner and
change rules in `alerts.rs` (`plan_*`, `first_sync_*`, `idle_sync_*`, `edits_and_deletes_*`, `the_earliest_*`,
`a_zone_change_*`) and the alert text in `model.rs`.

## REA evidence (official documentation, rendered)

- `ev_f447fda14b4d1062155f6a181882e8067aec24bd55317c81b9af7e61f6f622a0`: UNTimeIntervalNotificationTrigger (the trigger
  delivers after the time you specify; `init(timeInterval:repeats:)`; sample "fires in 30 minutes").
  https://developer.apple.com/documentation/usernotifications/untimeintervalnotificationtrigger
- `ev_e45c64e20de63d8e74af162a00fbcec799389a586b3f16087ef5d45fbca57902`: UNNotificationRequest (identifier, content,
  trigger; `add(_:withCompletionHandler:)`; `getPendingNotificationRequests`).
  https://developer.apple.com/documentation/usernotifications/unnotificationrequest
- `ev_5261eb285bd045276e9f8832278b92ff16f34fdb2a30e3fafd2e6e6ceae25c8f`: removePendingNotificationRequests(withIdentifiers:)
  (removes pending requests with matching identifiers; a non-repeating request whose trigger has been met is ignored).
  https://developer.apple.com/documentation/usernotifications/unusernotificationcenter/removependingnotificationrequests(withidentifiers:)

DocC reads (`python3 -I tools/parity/docc.py`): `usernotifications/untimeintervalnotificationtrigger`,
`usernotifications/unnotificationrequest`, `usernotifications/unusernotificationcenter/removependingnotificationrequests(withidentifiers:)`.

## Remaining gaps

- Calendar-component triggers (`UNCalendarNotificationTrigger`), location triggers and push triggers are not in the
  shell. Alerts use only the time-interval trigger (the matrix row stays `partial`).
- No repeating trigger is used: each alert is a one-shot request, re-planned as the window moves.
- Early alerts and due alerts are one request each; a reminder with both needs two of the 32 slots.
- No sound per request and no interruption level (the shell has no per-request sound).
- The 32-alert window means a reminder more than the window ahead gets its request only when the window reaches it
  (at most an hour later).
- Lists and priorities: no pure EventKit reminder model exists in `springboard/ui` or `springboard/core`. The EventKit
  "Reminders" row is `os` (app-internal store), and the reminder grant in `springboard/ui/src/calendar_access.rs` has
  no prompt or SDK entry point. Nothing was wired for them in this round.
- Recurrence: the Reminders app keeps its own repeat model (`src/repeat.rs`, daily to yearly). The EventKit
  recurrence rules (RRULE subset) live in `springboard/apps/calendar-desktop/src/calendar/recur.rs` and are app-internal;
  no shared model exists, so they were not wired.
- Location-based reminders (`springboard/ui/src/location_reminder.rs`) are not wired: no geofence monitor exists yet.
