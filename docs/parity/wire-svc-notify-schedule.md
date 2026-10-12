# Wire: notification-schedule service (calendar and interval triggers)

Branch `wire/svc-notify-schedule`, from `parity/integration`. Not built on the host: this round has no builds (the lead
builds once). Run `cargo test` for `bifrost-sdk` and the `springboard` shell crate before merging, and test on Midget.

Framework: User Notifications (https://developer.apple.com/documentation/usernotifications), matrix file
`docs/parity-matrix/data/user-notifications.json`, row "Notification triggers".

## What is now wired

A named shell service, `notification-schedule`, that schedules a user notification on a time-interval trigger or a
calendar trigger, repeating or once. The shell fires it through the normal notification path (banner, lock screen,
Notification Center, badge), so the user sees it like any other notification.

- Shell service: `springboard/src/shell/services/notification_schedule.rs` (`NotificationSchedule`, implements
  `SystemService`), registered in `springboard/src/shell/services/mod.rs`.
- Shell delivery: `springboard/src/shell/usernotif.rs`. New `Shell::usernotif_fire` (delivers a request whose trigger fired,
  replacing an earlier delivery with the same identifier). `usernotif_tick` uses it too. `Registry::check` is public and
  `permitted(app)` reads the `notifications` permission.
- SDK client: `springboard/sdk/src/notification_schedule.rs` (`Schedule`, `Trigger`, `add`, `remove`, `pending`, `reply`),
  one `pub mod` line in `springboard/sdk/src/lib.rs`.

Reuse of existing models, no new models:

- Time-interval trigger: the service hands it to `usernotif::Registry::add` with `delay_ms` and `repeats`, the same
  pending list and repeat grid the wire path already uses (`Registry::due`).
- Calendar trigger: the existing alarm model `clock::alarm::Alarm` (`regular_fire`, local civil time through
  `clock::offset_fn`, weekday mask with bit 0 = Sunday). Its `repeat` mask gives the weekdays; 0 means every day.

Protocol: no change. Apps use the service channel (`bifrost_app::service::call`, `ToShell::ServiceCall`), as documented in
`docs/bifrost-ui.md` "System services".

### Service methods (JSON)

- `add`: `{"id", "title", "subtitle"?, "body"?, "thread"?, "category"?, "badge"?, "trigger"}`.
  - `trigger` interval: `{"kind":"interval","seconds":N,"repeats":bool}`, N a whole number from 1 to 4294967.
  - `trigger` calendar: `{"kind":"calendar","hour":0..23,"minute":0..59,"weekdays":[1..7]?,"repeats":bool}`. Weekdays
    run from 1 = Sunday to 7 = Saturday; missing or empty means every day.
  - Answers `{"added": id}`. Refusals are service errors (no permission, empty identifier, unknown category, text over
    the wire limits, too many pending, no matching date).
- `remove`: `{"ids": [...]}`; missing or empty removes every pending request of the app. Answers `{"removed": n}`.
- `pending`: answers `{"pending": [ids]}` (interval and calendar requests).

Rules: an identifier replaces the pending request of either kind; a delivered one with the same identifier is replaced by
the new delivery. Pending is capped at 64 per app (`MAX_PENDING`, shared by both kinds). Every method needs the
`notifications` permission in the app manifest.

### How a user reaches it

A user does not call this directly. An app schedules a reminder (for example a daily standup at 09:15 on weekdays), and
the notification shows up at that time in the normal notification UI. Per-app mute and Do Not Disturb apply as they do
to any other notification.

## How to verify on the test PC (Midget)

1. Build and publish the shell per `docs/bifrost-pc-workflow.md`; update Midget with `bifrost pc midget ota` until
   `phase=accepted`, then take a screenshot with `bifrost pc midget screen PATH`.
2. Deploy a small test app with the `notifications` permission. From its `App::service_reply` hook, call
   `notification_schedule::add` with a calendar trigger about two minutes ahead (`Schedule::calendar(..., repeats: false)`),
   and an interval trigger of 60 seconds with `repeats: true`.
3. Check `notification_schedule::pending` replies with both identifiers. Add a request with the same identifier and check
   that only one is pending.
4. Wait for the time. Expect one banner per due time; the repeating one comes again each minute until removed with
   `notification_schedule::remove(ctx, &[id])`. Check the lock screen and Notification Center entries.
5. Read the shell log (`bifrost pc log`) for errors from the service.

Not yet verified on Midget or the host: nothing in this branch has been run.

## REA evidence

- `ev_d1491e1889c0049021d778bc26ff2b4150ec53a37d942cb2daeda04b4363a21d`: rendered page
  https://developer.apple.com/documentation/usernotifications/uncalendarnotificationtrigger (creating a calendar trigger
  from date components with `repeats`, `nextTriggerDate()`, `dateComponents`).
- `ev_117d18d84af6eb801ef3a198c6ea9e2ca69d9d98b9e4342b042da4cce20375e7`: rendered page
  https://developer.apple.com/documentation/foundation/datecomponents/weekday. The rendered text was not checked for the
  numbering; the 1 = Sunday convention comes from the Gregorian calendar and is an assumption until that page is read.
- Official DocC (`tools/parity/docc.py`): `usernotifications/uncalendarnotificationtrigger` and its
  `init(dateMatching:repeats:)`; `usernotifications/untimeintervalnotificationtrigger` (`init(timeInterval:repeats:)`).
  The official pages state no minimum interval for repeating triggers, so none is enforced beyond 1 second.

## Gaps left

- Calendar trigger: only hour and minute (and weekdays). Seconds, day of month, month and year components are not
  supported; the time is the start of the minute. No time-zone rule of its own: the shell's current local offset is used,
  so a daylight-saving change moves the next time only after the shell next reads the zone.
- Pending calendar requests are not kept across a shell restart. The interval ones are not either.
- Timing depends on the shell tick. A shell that sleeps with no frames may fire a trigger late; there is no wake-up for
  either kind yet (`usernotif` has none). Do not rely on second-level accuracy.
- No `nextTriggerDate()` for the app, no location or push triggers, no sound, attachments or `userInfo`.
- The service does not appear in `docs/bifrost-ui.md`; the lead should add it to the "System services" list and set the
  "User notifications" note's "no repeating triggers" line to the current state.
- `docs/parity-matrix/` is not changed here; the row "Notification triggers" stays `partial`.
