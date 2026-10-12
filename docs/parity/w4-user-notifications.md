# Parity wave 4: User Notifications (settings and Focus query)

Framework: User Notifications (`docs/parity-matrix/data/user-notifications.json`). This wave takes the one `os` row,
"Per-app notification settings and Focus". The shell already had the per-app mute, the notification permission and Do
Not Disturb; apps had no way to read them. Apps now get a settings query over the existing shell state.

## What was built

- **Wire** (`springboard/proto`): `ToShell::NotifySettings { request }` (tag 0x14a) and `ToApp::NotificationSettings
  { request, settings }` (tag 56). `NotifSettings` is one flags byte: authorized, alert, sound, badge, focus. Bits
  above 0x1f are rejected on decode.
- **Shell** (`springboard/src/shell/usernotif.rs`, `apphost.rs`): the query is answered for every app, also one without
  the `notifications` permission. `settings_for(permitted, muted, focus)` is the pure rule:
  - denied: nothing allowed, Focus still reported;
  - granted and not muted: authorized, alert, sound and badge on;
  - granted and muted: authorized only;
  - Focus is reported for any app.
- **SDK** (`springboard/sdk/src/usernotif.rs`, `runtime.rs`): `usernotif::settings(ctx) -> u32` answers with
  `Event::Settings { request, settings }` through `App::notification`.

## Official API mirrored

- `UNNotificationSettings` (authorizationStatus, alertSetting, soundSetting, badgeSetting). Doc:
  https://developer.apple.com/documentation/usernotifications/unnotificationsettings
- `UNUserNotificationCenter.getNotificationSettings(completionHandler:)`. Doc:
  https://developer.apple.com/documentation/usernotifications/unusernotificationcenter/getnotificationsettings(completionhandler:)
- `UNAuthorizationStatus` (`denied` and `authorized` are produced; `notDetermined`, `provisional`, `ephemeral` are not).
  Doc: https://developer.apple.com/documentation/usernotifications/unauthorizationstatus
- Focus status: BifrOSt reports Do Not Disturb as a flag in the same answer. Apple's focus status is a separate
  framework call and is not mirrored by name.

## REA evidence

- `ev_63714303f2e9e3ea988835f525a082f0e265158d2f895f7f16e17de6ebb46505`: UNNotificationSettings
- `ev_7395e15edeba6c09a0f30604147a0c13458a89657f4e33ede21aca7b4be50f21`: getNotificationSettings(completionHandler:)
- `ev_7d88bf0ff2a28f851caf48c8f014e0f41e1f170b8218bc8dc617e31bc0337e8e`: UNAuthorizationStatus

## Limits (why partial)

- Authorization is a boolean from the manifest grant. There is no prompt, so `notDetermined` never occurs and
  `provisional` and `ephemeral` do not exist.
- Sound and badge follow the app's mute state. The per-app Sounds switch in Settings only exists for built-in apps,
  so an external app's sound cannot be queried separately.
- Alert is one flag. It is not split into Notification Center, Lock screen and banner settings.
- Settings and Focus are a point-in-time answer. There is no change callback.
- The query also answers the status part of the "Asking permission to use notifications" row. That row stays partial
  because the prompt and its options are still missing.

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| user-notifications | Per-app notification settings and Focus | partial | springboard/sdk/src/usernotif.rs (settings), springboard/src/shell/usernotif.rs (settings_for), springboard/proto/src/rich.rs (NotifSettings) |

## Not built in this wave

- Communication notifications, remote push, background push, web push: each needs a push or sender-identity service
  BifrOSt does not have yet. Nothing pure-Rust in this wave covers them.

## Tests

Added: `notification_settings_round_trip_every_flag` and `notification_settings_reject_unknown_flag_bits` (proto),
`settings_event_carries_flags_and_request` (sdk), and three shell tests covering denied, granted, muted and Focus
answers. Builds and tests were not run, per the wave rules.
