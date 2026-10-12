# Wire 6: user notification parity (first-time prompt, alert style)

Branch `wire6/notifications`, from `main` (0.5.43). Not built on the host: this round has no builds (the lead builds
once). Run `cargo test -p springboard --lib usernotif notify` and `cargo test -p springboard --lib notify_page` before
merging, and check on Midget.

Framework: User Notifications (https://developer.apple.com/documentation/usernotifications). Matrix files:
`docs/parity-matrix/data/user-notifications.json` and `user-notifications-ui.json`.

## What is now wired

1. **First-time permission prompt.** The first request an app with the `notifications` permission makes is held, and
   the shell shows one alert: "“App” Would Like to Send You Notifications", with Don't Allow (default, left) and Allow
   (right). The answer is stored once per app and the app is never asked again. Allow clears the app's mute (its
   Allow Notifications switch in Settings is on); Don't Allow sets it. The held requests then get their status:
   `Ok` after Allow (delivered, scheduled or refused as usual), `NotPermitted` after Don't Allow. If the alert cannot be
   built, the answer is Don't Allow.
   - Model: `bifrost_ui::permission_question` (`AskCenter`, `PermissionQuestion`, topic `notifications`), the same
     model the privacy sheet uses. Card: the shell `AlertCard`.
   - Files: `springboard/src/shell/notify/permission.rs` (new: `Sheet`, `NotifTopic`, `apply_answer`),
     `springboard/src/shell/usernotif.rs` (`Registry::hold`, `take_held`; `usernotif_hold`, `usernotif_ask_tick`,
     `usernotif_ask_event`, `usernotif_answer`, `usernotif_submit`), `springboard/src/shell/notify/store.rs` (an
     `[asked]` section in `notifications.ini`, `is_asked`, `set_asked`), shell hooks in `springboard/src/shell/mod.rs`
     (modal touches, draw, damage, the tick).
2. **Alert Style per app** (Settings > Notifications > one app). The old Banners switch and Temporary/Persistent rows
   are one group, ALERT STYLE, with None, Banners and Alerts (the `UNAlertStyle` cases none, banner and alert).
   - None: the notification is stored, badged and listed in Notification Center, but no banner and no screen wake.
   - Banners: the banner goes away after five seconds (the default, as before).
   - Alerts: the banner stays until it is tapped or swiped away (`Banner::persistent`).
   - Stored in the existing per-app preference (`notif.<app>`, `AppNotif`); `AppNotif::style` maps the two flags.
   - Files: `springboard/src/shell/notify_page.rs` (`ALERT_STYLES`, `AppNotif::style`, `AppField::Style`, `app_spec`),
     `springboard/src/shell/notify/mod.rs` (`notify_id` checks `app_notif(app).banners`, `start_banner`, `app_notif`),
     `springboard/src/shell/notify/banner.rs` (`persistent`).

## How a user reaches it

- Alert: an app that has the `notifications` permission and posts its first notification. The alert appears over the
  screen; the choice is final (change it later in Settings > Notifications > the app, Allow Notifications).
- Alert Style: Settings > Notifications > the app > ALERT STYLE.

## How to verify on the test PC (Midget)

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`. Update Midget per `docs/bifrost-pc-workflow.md`.
2. Install a test app with the `notifications` permission that calls `set_categories` (optional) and then
   `add` with a request (`title` "Test", no delay). Expect the alert "“<app>” Would Like to Send You Notifications".
3. Tap Don't Allow. Then post again from the app: no banner, but the notification is in Notification Center, and
   Settings > Notifications > the app shows Allow Notifications off. Its `add` replies `NotPermitted`.
4. Turn Allow Notifications on. Post again: a banner appears. Remove the app's `notif.*` preference or reinstall it to
   test the first-time alert again (the store is `notifications.ini`; the `[asked]` section holds the answer).
5. Settings > Notifications > the app > ALERT STYLE: None, then post: no banner. Alerts, then post: the banner stays
   until tapped. Banners: the banner leaves after five seconds.
6. Read the shell log (`bifrost pc log`) for errors. Host unit tests: `store::tests::the_permission_answer_is_kept_once_and_apart_from_the_mute`,
   `permission::tests::*`, `usernotif::tests::requests_held_for_the_first_alert_come_back_with_the_answer_in_order`,
   `notify_page::tests::the_alert_style_*`, `banner::tests::an_alerts_banner_rests_until_it_is_dismissed`.

## REA and official evidence

- Rendered page, `https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications`
  (`tools/parity/rea-page.sh`): `ev_14ae8678733a3ad0e084bb6b7c732de7cac49cb8bd70112c371717c4d977fd4e`. It says the first
  authorization request prompts the person and records the answer, that later requests do not prompt, and that the
  request should be made in context. It also describes provisional authorization (quiet delivery to the history).
- Rendered page, `https://developer.apple.com/documentation/usernotifications/unalertstyle`:
  `ev_87b7e8b275dd3dd6f0bfbc394415cde016c71a0c685b3a5dc9c69fbe93e72c7d`. Cases `none`, `banner`, `alert`; macOS 10.14+.
- Official DocC (`tools/parity/docc.py`): the same two pages. The "Alert Style" rows follow `UNAlertStyle`; the exact
  rendering of a persistent alert on the desktop is BifrOSt's choice (the official page says only "Modal alerts").

## Remaining gaps

- **Notification action buttons in banners and Notification Center** are not drawn. The cards have no action row;
  `Registry::actions` exists, but no card shows it. This is the largest open item of the row "Custom actions and
  response handling" and of "Notification categories and actions".
- **Grouping by thread.** Notification Center groups by app only. `NotifRequest.thread` reaches the registry but is not
  stored on the store's `Notification`, so no thread sub-group exists. The "Notification Grouping" option (Automatic,
  By App, Off) is stored but not applied to the groups.
- **Provisional delivery** (quiet, history only) needs a proto field, which this round does not change. Not wired.
- **Foreground presentation (`willPresent`)**: an app cannot choose how a notification shows while it is in front.
  Not wired.
- **Scheduled requests from the `notification-schedule` service** do not go through the first-time alert. They are
  still stopped by the Allow Notifications switch (the mute), so a denied app gets no banner from them either.
- **Lock Screen and Notification Centre switches** of the app page are stored but not enforced.
- **Authorization status query** (`UNNotificationSettings.authorizationStatus`) is still the manifest answer plus the
  mute; the first-time answer is not reported as a "not determined" state to the app.
- Held requests of an app that quits before the answer stay held until the app is uninstalled (`forget_app`) or the
  next answer. No timeout.
- No Midget verification, and no host build for this round.

## Matrix rows

Rows are in `docs/parity-matrix/data/user-notifications.json` (and `user-notifications-ui.json` where noted). Each stays
`partial`; this round does not change the matrix.

| slug | feature (as written in the matrix json) | BifrOSt path |
|---|---|---|
| user-notifications | Asking permission to use notifications | springboard/src/shell/notify/permission.rs (`Sheet`, `apply_answer`); springboard/src/shell/usernotif.rs (`usernotif_hold`, `usernotif_ask_tick`, `usernotif_answer`) |
| user-notifications | Per-app notification settings and Focus | springboard/src/shell/notify_page.rs (`ALERT_STYLES`, `AppNotif::style`, `AppField::Style`); springboard/src/shell/notify/mod.rs (`notify_id`, `app_notif`); springboard/src/shell/notify/banner.rs (`persistent`) |
| user-notifications | Notification display (banners, lock screen, Notification Center) | springboard/src/shell/notify/banner.rs (Alerts style rests until dismissed); springboard/src/shell/notify/store.rs (`[asked]` section) |
| user-notifications-ui | Custom actions and response handling | not wired this round: no action row on banners or cards (see gaps) |
