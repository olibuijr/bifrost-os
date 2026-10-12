# Connect: Reminders app to the system services

Branch `connect/reminders`, from `parity/integration`. Owner scope: `springboard/apps/reminders-desktop/` (the app, its
tests and its CHANGELOG entry for 1.10.0). No builds were run in this round; the lead builds once. Earlier wiring is in
`docs/parity/wire-app-reminders.md`.

## What is connected

| Service | Matrix slug | Feature (as in the matrix) | App code | Status |
|---|---|---|---|---|
| `notification-schedule` | `user-notifications` | Notification triggers | `src/alerts.rs` (`apply`, `interval_secs`), `src/main.rs` (probe in `tick`) | Wired: due and early alerts are interval requests on the service, with the older `usernotif` path as the fallback |
| `dock` | `appkit-user-interface` (Menus, Cursors, and the Dock, NSDockTile) | Dock tile badge | `src/main.rs` (`tick`, `service_reply`) | Wired: the badge goes through `dock_tile::set_badge`; the older `ToShell::SetBadge` is the fallback |
| `widgets_timeline` | `widgetkit` | Timeline updates | `src/system.rs` (`today_entries`, `reload_today`), `src/main.rs` (`tick`, `service_event`) | Wired: a Today timeline for Notification Centre |

### Due alerts (`notification-schedule`)

- On the first tick the app sends one `pending` probe to `notification-schedule`. Alerts wait for the answer. An answer
  with a reply means the service is there, and alerts go through `notification_schedule::add` and `remove`. An error
  (an older shell answers "no service ...") switches the app to the `usernotif` path it used before, so alerts keep
  working.
- Each alert is a one-shot interval trigger: `Schedule::interval(id, title, seconds, false)`. The delay is rounded up
  to whole seconds (`interval_secs`), so an alert is never early, and clamped to the service's limit.
- A reset (first sync after launch) sends `remove` with no ids, which clears every pending request of the app, then
  adds the plan. Edits send `remove` with only the changed ids and then `add`. Calls happen only when the plan changes,
  as before.
- Replies to the `add` and `remove` calls are not tracked one by one. A refusal (for example the 64-request cap) is
  dropped; the next re-plan (within the hour, or on the next data change) sends the plan again.

### Dock badge (`dock`)

- The badge count is unchanged: open reminders due today or earlier (`model::due_count`). The label goes through
  `model::badge_label` (99+ above 99).
- The request id is remembered with its label. If its reply is an error, the same label is sent with
  `ctx.set_badge`, the message the shell has always had. Other replies are ignored.

### Today widget (`widgets_timeline`)

- The timeline kind is `today`. Its entries: a summary row dated now ("Today", "N due", "Nothing due" or "1 due"),
  then up to eight open reminders due today or earlier, soonest first. Overdue ones are dated now and read "Overdue";
  the others show their due time. Completed and undated reminders are left out.
- The timeline is sent when the app's data changes (its generation changes), once on the first tick, and again when
  the shell sends the `reload` event for `today`. Its policy is `after` the next local midnight, so the shell asks for
  a new one then.
- An older shell without `widgets_timeline` answers with an error; the app ignores it. The widget is then empty.

## How a user sees it

1. Alerts: add a reminder with a due time, and an Early alert if wanted. Close the Reminders window. The banner appears
   at the due time (and the early time), as before, but the shell now holds the alerts itself.
2. Dock: Reminders shows a red badge on its Dock icon with the number of open reminders due today or earlier. The
   count drops when they are completed or moved to a later day.
3. Notification Centre: the Today widget lists today's open reminders and the overdue ones, with the summary row on top.
   It changes when a reminder is added, edited, completed or deleted, and at midnight.

## How to verify on the test PC (Midget)

Midget only; the VM is not used.

1. Build and publish as usual (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`). Check the version
   with `bifrost pc midget state`.
2. Alerts through the service: `bifrost pc midget open-app is.olibuijr.reminders.desktop`. Add "Wire test" due in about
   two minutes with an Early alert of one minute. Close the Reminders window. Expect the banner at the early time and at
   the due time. Check `bifrost pc log` for `notification-schedule` requests and no errors.
3. Fallback: on a shell without the service (an older build), the same reminder still gives its banner through
   `usernotif`.
4. Dock badge: with two reminders due today, the badge shows "2" on the Reminders Dock icon. Complete one; it shows "1".
   With none, no badge.
5. Today widget: open Notification Centre. Expect a "Today" row with the count and the reminders. Add one; the row
   updates within a shell tick. Capture with `bifrost pc midget screen /tmp/reminders-today.png`.

Host tests (not run here): `cargo test -p bifrost-reminders`, which covers `system::tests` (the timeline rows, the
midnight policy and the `widgets_timeline` reload call) and `alerts::tests` (interval rounding, the service reset and
remove calls, the legacy fallback path).

## Gaps

- Repeating triggers: the service's `repeats` flag is not used. A repeating reminder gets its next alert when it is
  completed, through the re-plan, which matches the Reminders behaviour of advancing the due date. A calendar trigger
  with `repeats` would duplicate that and would not fit monthly or yearly repeats (the service has hour, minute and
  weekdays only).
- Calendar triggers (`UNCalendarNotificationTrigger` style, date components) are not used for the same reason; alerts
  stay interval requests planned by the app.
- No sound, interruption level or per-request category on alerts (the service has none for requests of this kind).
- Pending requests are not kept across a shell restart; the app re-plans on its next start (first sync resets and
  re-adds).
- A refused `add` or `remove` is not retried per request (see "Due alerts" above).
- The Dock badge is a count only; the `dock` service's progress and bounce are not used.
- The Today widget has one kind and no widget families, no interactivity and no relevance score. The shell's widget
  row is the only place it shows (no lock-screen strip).
- The `min_shell` in `manifest.toml` is unchanged (0.5.7). The services used here come from the same system round as
  the other wiring; the lead should confirm the shell version that first has `notification-schedule`, `dock` and
  `widgets_timeline`, and raise `min_shell` if needed.
- The probe waits for a reply before alerts are planned. If the shell never answers the probe, no alerts are planned in
  that session.
- The `docs/bifrost-ui.md` "System services" list and the matrix rows (`user-notifications`, `appkit-user-interface`,
  `widgetkit`) are not changed here; the lead updates them.
