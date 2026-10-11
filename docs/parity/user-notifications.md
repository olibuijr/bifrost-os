# User Notifications: repeating time-interval triggers

Framework: User Notifications (https://developer.apple.com/documentation/usernotifications), matrix file
`docs/parity-matrix/data/user-notifications.json`.

## What was built

A repeating time-interval trigger for notification requests. An app asks for a request that fires every
`interval` milliseconds until it removes the request, which mirrors `UNTimeIntervalNotificationTrigger` created
with `init(timeInterval:repeats:)`. A repeat replaces the earlier delivery with the same identifier, as a delivered
request with an identifier replaces the earlier one.

- SDK (`springboard/sdk/src/usernotif.rs`): `Request::repeats(bool)`, used with `Request::after_ms`. Without a delay
  the request is delivered once, now.
- Wire (`springboard/proto/src/rich.rs`): `NotifRequest.repeats`, one flag byte after the badge.
- Shell (`springboard/src/shell/usernotif.rs`): `Registry::due` keeps a repeating request pending on its own grid
  (the first trigger date after the current time, so a shell that slept past several triggers delivers once and
  does not replay them). `Registry::take_delivered` is now public, and each delivery replaces the previous one with
  the same identifier.

Apple API mirrored: `UNTimeIntervalNotificationTrigger`, `init(timeInterval:repeats:)`, `timeInterval`.
The shell's next due time is the counterpart of `nextTriggerDate()`, but the app does not get it.

The official page does not state a minimum interval for repeating triggers, so none is enforced.

## REA evidence

- `ev_454100d7137daf73e5c4f95925a19aff9b0d25cefe7e9466b0a5e283bfd56bee`: rendered page
  https://developer.apple.com/documentation/usernotifications/untimeintervalnotificationtrigger (topics
  `init(timeInterval:repeats:)`, `nextTriggerDate()`, `timeInterval`).

## Tests

- SDK: `request_builder_fills_every_field` (updated), `repeats_is_off_until_asked_and_keeps_the_interval`.
- Proto: `notification_messages_round_trip` (the request carries `repeats: true`).
- Shell: `repeating_trigger_fires_on_its_grid_until_removed`, `repeats_without_a_delay_deliver_once`.

Not run: this round has no builds (rule of the parity wave). Run `cargo test` for the `proto`, `sdk` and shell crates
before merging.

## Matrix rows closed

| Framework slug | Feature (as in data/user-notifications.json) | New status | BifrOSt path |
|---|---|---|---|
| user-notifications | Notification triggers | partial (unchanged) | springboard/sdk/src/usernotif.rs (`after_ms`, `repeats`), springboard/src/shell/usernotif.rs (`Registry::due`) |

The row stays `partial`: calendar triggers (`UNCalendarNotificationTrigger`), location triggers and push triggers are
still missing. The `gaps` entry "Calendar and location triggers: only a time-interval delay exists; no repeating or
region-based notifications" is now only partly true; the repeating part is closed. The line in `docs/bifrost-ui.md`
("no repeating triggers") is stale and should be updated by the lead.

## Not built

The other gaps in the matrix (calendar and location triggers, sounds, attachments, `willPresent`, the authorization
prompt) were not built in this round. The calendar trigger needs a local time-zone clock in the shell, and the
attachment needs a rendering path.
