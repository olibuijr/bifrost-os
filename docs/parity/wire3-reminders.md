# Wire 3: Reminders (1.11.0) wires the first-use alert tip

Branch `wire3/reminders`, on top of main (release 0.5.38). The change touches `springboard/apps/reminders-desktop/`
and this note. Nothing was built or run (the lead builds once). Host tests were written by hand, not run.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/tipkit.json`) | BifrOSt path |
|---|---|---|
| tipkit | Inline tip view and style | `springboard/apps/reminders-desktop/src/app.rs` (`AlertHint`, `Reminders::with_tips`, `tip_sections`, `dismiss_tip`; `TipView::new(&AlertHint, TipViewStyle::Mini)`), `view.rs` (`draw_tip`), `main.rs` (`tip_store`) |

What the user sees: in the details of a reminder that has no due date, a card under the Due row reads "Get an alert"
and "Set a due date and a time. The alert still shows when Reminders is closed." The card is the shared `TipView`
(Mini style: title and message, no actions). Its state is a `Datastore` in the app's data folder (`tips.txt`), with
an in-memory fallback when that file cannot be read.

- The tip is drawn only in the details form of a reminder without a due date.
- It is recorded as shown the first time it is drawn in a session.
- Tapping the card dismisses it for good (`invalidate`). So does tapping Due to set a date, and it stays dismissed
  when the date is cleared again.

Also in this round, the main content background is `CONTENT_BG` (white in Light, dark grey in Dark), not the pure
black of `DETAIL_BG`. This covers the window fill (`chrome.rs`, `background`) and the unselected list row fill
(`view.rs`, `draw_item`). The selected row keeps `CARD_BG`.

## How a user reaches it

1. Open Reminders and press New.
2. The details form opens with the tip card under Due.
3. Set a due date to dismiss it. Tapping the card also dismisses it.

## How to verify on the test PC (Midget)

The owner rule is to test on Midget only (no VM).

1. Publish and update as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Check the
   version with `bifrost pc midget state` (expect Reminders 1.11.0).
2. Open Reminders with `bifrost pc midget open-app is.olibuijr.reminders.desktop`, then press New. Expect the
   "Get an alert" card under Due. Capture it with `bifrost pc midget screen /tmp/reminders-tip.png`.
3. Tap the card, or tap Due. Expect the card to go away. Cancel, press New again, and expect no card.
4. Light and Dark: the content background should be white and dark grey, with no black panel behind the list.
   Compare with `bifrost pc smoke reminders-desktop --light` on the host.

Host tests (not run): `cargo test -p bifrost-reminders` covers `first_use_alert_tip_shows_in_details_until_a_due_date_is_set`,
`tapping_the_alert_tip_dismisses_it` and `without_a_tip_store_the_details_form_has_no_tip_row` in
`src/actions_tests.rs` (the compiled test module). Expected values by hand: a new reminder has no due date, so the
tip row is in `form_rows`; setting the date dismisses it; clearing the date leaves it dismissed; with no store there
is no row.

## REA evidence (official documentation, rendered)

- `ev_fb74d28b460cedb26072b3e3ae4f03605f647c615ade4f90ed3ca460b10e86f5`: TipView (an inline tip view with an optional
  arrow; the Tip protocol supplies the content; init(_:arrowEdge:action:); popoverTip; "use TipUIView and TipNSView when
  adding tips to UIView and NSView hierarchies"). https://developer.apple.com/documentation/tipkit/tipview
- DocC read (`python3 -I tools/parity/docc.py tipkit/tipview`): the TipView symbol and its three initialisers.

## Remaining gaps

- The tip has no arrow or anchor. It is an inline card under Due, not a popover on the control, because the SDK has no
  tip anchor (the matrix row "Popover tip" stays Partial).
- Tip rules and frequency are the defaults (`Immediate`, no rules). There is no per-session limit beyond the
  dismissal.
- EventKit alarms: the details form has only the relative Early row. Absolute alarms are not in the app. The
  matrix row "Alarms" stays Partial.
- EventKit priority: the app keeps its own 0 to 3 scale (None, Low, Medium, High). EventKit's numeric scale (1 to 4
  high, 5 medium, 6 to 9 low) has no model in `springboard/ui`, and the store file has no EventKit field, so it is not
  mapped.
- EventKit recurrence rules (`EKRecurrenceEnd`): the Repeat row has no end date. The RRULE model in
  `springboard/apps/calendar-desktop/src/calendar/recur.rs` is app-internal and not shared.
- Location-based reminders (`springboard/ui/src/location_reminder.rs`) are not wired: the shell has no geofence
  monitor, and the models' `LocationAlarm` has no delivery path.
- Reminder access (`springboard/ui/src/calendar_access.rs`): the app keeps its own file store, so no permission
  prompt is shown.
- `springboard/apps/reminders-desktop/src/tests.rs` is not included by any module (only `actions_tests.rs` is compiled),
  so its tests do not run. Moving them in is a separate change.

## Matrix rows

| Slug | Feature (exactly as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| tipkit | Inline tip view and style | `springboard/apps/reminders-desktop/src/app.rs` (`AlertHint`, `TipView`, `TipViewStyle::Mini`); `view.rs` (`draw_tip`) |
