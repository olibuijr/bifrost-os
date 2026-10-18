# Notifications: shell side

Code: `springboard/src/shell/notify/` (store, banners, lock screen cards, Notification Center), `usernotif.rs`
(the `UNUserNotificationCenter` registry on the shell side), `services/notification_schedule.rs` (scheduled requests).

## Store and display

- Every notification is kept in `notifications.ini` (`notify/store.rs`, the last `KEEP` entries, written through a
  temporary file and a rename).
- A banner slides down from the top for five seconds. The lock screen and Notification Center show cards grouped by app
  (`notify/list.rs`, `notify/center.rs`).
- Cards draw the action buttons a category declares, with their icons. A category may give a custom dismiss action.
- A PNG attachment shows as a thumbnail on the card.
- Hidden previews: a category, or an app, can hide the text of its notifications. `previews_hidden` in `notify/mod.rs`
  decides what banners and lists reveal.
- Each delivery is logged under the `notify` logger (`notify/mod.rs`).

## UNUserNotificationCenter side (`usernotif.rs`)

- The registry keeps the categories each app registered, requests waiting for their trigger, the delivered requests and
  answers that wait for an app that is not running. Identity is always the app id of the connection, never a value in
  the message.
- `willPresent`: when an app in front has asked to see its own notifications, the shell asks it first and holds the
  notification until the answer (`usernotif.rs`, `presentation_plan`). No answer in time hides the notification. The
  answer is cut to what the grant and Mute allow.
- Pending requests of one app are capped (64, as the reference centre allows). A request with the same identifier
  replaces the earlier one.

## Scheduled requests (`services/notification_schedule.rs`)

- Interval triggers go to the registry's time-interval path, with the repeat grid.
- Calendar triggers keep their next time from the alarm model (local civil time), so they follow the zone's offset.
  Weekdays run 1 (Sunday) to 7; an empty list means every day.
- Both kinds share one pending list per app, saved to `pending-notifications.ini` and read back at start.
- Every method requires the `notifications` permission.

## Gaps

- Focus modes (`notify/modes.rs`) are a separate model and are not described here.
