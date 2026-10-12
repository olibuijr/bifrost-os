# Wire 5: Clock (time parity)

Branch `wire5/clock`, from `main` (0.5.42). App `springboard/apps/clock-desktop/` (`bifrost-clock` 1.4.0, id
`is.olibuijr.clock.desktop`). No builds were run this round: the lead builds once. Run `cargo test` in
`springboard/apps/clock-desktop` and check the Midget steps below before merging.

Read first: `docs/parity/connect-clock.md` (the previous round: World Clock widget, Dock bar and bounce, and why alarms
are not scheduled through `notification-schedule`), and `docs/parity/wire-svc-notify-schedule.md`.

## Checked before wiring (already works, left alone)

| Focus item | State | Where |
|---|---|---|
| Alarm repeat days | Wired in the shell, not the app. The shell's headless Clock service fires alarms on their repeat days (`regular_fire`, weekday mask bit 0 = Sunday), so they ring with the app closed. | `springboard/src/shell/clock/alarm.rs`; the editor's Repeat page is `DAY_ROWS` in `springboard/apps/clock-desktop/src/clock/mod.rs` |
| Snooze | The alarm model has a Snooze switch (`Alarm::snooze`, `snoozed_until`), the editor's Snooze row, and the card shows "Snoozed, ...". The snooze itself runs in the shell's alert. | `springboard/apps/clock-desktop/src/clock/mod.rs` (`Target::Toggle(2)`, `draw_alarms`), `springboard/src/shell/clock/mod.rs` |
| World clock with time zone search | Wired. The Add City sheet searches the city dataset by name (case and accent folded, names starting with the query first). | `springboard/apps/clock-desktop/src/clock/world.rs` (`search`, `move_city`), `springboard/src/shell/clock/tz.rs` (`CITIES`) |
| Timers | Wired: several concurrent timers, presets, Dock bar and bounce (see `connect-clock.md`). | `springboard/src/shell/clock/timer.rs` (shell model), `springboard/apps/clock-desktop/src/system.rs` |

## What is now wired

- **Stopwatch: Copy Laps.** A new toolbar item on the Stopwatch tab, "Copy Laps" (symbol `square.and.arrow.up`). It is
  greyed out until the stopwatch has started. Pressing it puts the lap table on the general pasteboard as plain text:

  ```
  Lap<TAB>Split<TAB>Total
  1<TAB>00:03,61<TAB>00:03,61
  2<TAB>00:01,39<TAB>00:05,00
  ```

  Laps are listed oldest first, with the same `MM:SS,hh` format the table shows. The lap in progress is the last line,
  as it is in the table. The text reaches other apps through the general board (`NativePasteboard::set_string`, the
  `ToShell::PasteboardSet` message the shell already accepts).
  - `springboard/apps/clock-desktop/src/clock/stopwatch.rs`: `Stopwatch::export_text(t)` (None before a start).
  - `springboard/apps/clock-desktop/src/clock/mod.rs`: `BAR_EXPORT` (id 12), `Target::Export`, the toolbar item in
    `bar()`, the hit and pressed mapping, the activation that sets `pending_export`, and `Clock::take_export()`.
  - `springboard/apps/clock-desktop/src/main.rs`: after a touch, `take_export` hands the text to
    `bifrost_app::pasteboard::NativePasteboard::general().set_string(ctx, ...)`. The Clock hosts no copy logic itself.
- **Window background.** The content area in Dark is `CONTENT_BG` (the dark grey), not `DETAIL_BG` (pure black). The
  draw code fills the whole pane with it.

## Tests

- `stopwatch.rs::export_lists_laps_oldest_first_under_a_header`: start at 0 ms, lap at 3610 ms, export at 5040 ms. Expected
  text worked out by hand from `format` (3610 ms = `00:03,61`, 1430 ms = `00:01,43`, 5040 ms = `00:05,04`).
- `mod.rs::copy_laps_queues_the_lap_table_once`: drives the buttons with `tap`, so each press acts at its Up time (press
  time + 20 ms). Start acts at 20 ms, lap at 3630 ms (mark 3610), stop at 5020 ms (banked 5000). The export after the
  stop gives lap 2 as 1390 ms (`00:01,39`) and the total 5000 ms (`00:05,00`), independent of the time it is read.
  The text is handed over once.

## How a user reaches it

1. Open Clock, choose the Stopwatch segment of the toolbar.
2. Press Start, press Lap a few times, press Stop.
3. Press Copy Laps in the toolbar (right of the segments). The button is disabled until the stopwatch has started.
4. Paste into any text field. The paste shows the header and one line per lap.

## How to verify on the test PC (Midget)

Midget only; the lab VM is not used.

1. Update the app: `ssh midget` then `/usr/bin/springboard --install-store-app is.olibuijr.clock.desktop` if the store copy
   is older than 1.4.0 (publish first with `bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`).
2. Open Clock on the Stopwatch tab. Copy Laps is greyed out. Press Start: it turns on. Press Lap twice, then Stop.
3. Press Copy Laps. Open a text field in another app, paste, and check the text has the header and two lap lines.
4. Press Copy Laps while the watch runs and check that the lap in progress is the last line.
5. Screenshot: `bifrost pc midget screen PATH`. Check the toolbar item sits right of the segmented control and in Dark the
   window background is the grey content colour (not black). Use `bifrost pc midget appearance dark` and `light` for both.
6. Read `bifrost pc log` for errors from the Clock.

Not yet verified on Midget or the host: nothing in this branch has been run.

## Not wired and why

- **Alarms through `notification-schedule` (repeat days as calendar triggers).** Not done. The shell already fires every
  alarm, including repeat days, through its headless Clock service and its own alert. A second schedule through
  `notification-schedule` from the app would show each alarm twice while the app is open. The real gap is on the shell
  side: `notify_alarm` looks the Clock up with `apphost::slot_of("is.olibuijr.clock")` and drops the banner when that id is
  not installed, and the shipped id is `is.olibuijr.clock.desktop`. That fix belongs to `springboard/src/shell/`, which
  this round does not touch. Once it is settled, the app could add a `notification-schedule` fallback with the same
  identifier per alarm (the same identifier replaces the pending request), and that needs the `notifications` permission in
  the manifest.
- **Snooze as a notification action.** Not done. The snooze button belongs to the shell's alarm alert (`springboard/src/shell/clock/`),
  and the alarm notification is posted by the shell with no category. Adding a "Snooze" action needs a shell change.
- **Stopwatch export as a file or share sheet.** Not done. Copy to the pasteboard is the only export; no file format and
  no share sheet.
- **World clock search** is by city name only; no search by time zone abbreviation or offset.

## REA evidence

No new official page was relied on this round. The copy goes through the existing pasteboard wiring, which cites:

- `ev_a56d232a88893f8c75a787655b47c8fcee754bfbfca75a880b0ce1fff09c3d10`: rendered page
  `https://developer.apple.com/documentation/appkit/nspasteboard` (`setString(_:forType:)`, `clearContents`, the general
  pasteboard shared by all apps), from `docs/parity/wire-svc-pasteboard.md`.

## Matrix rows

Rows this round moves toward Parity (the status in the matrix stays `partial` until the Midget check passes):

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| application-services | Pasteboard (NSPasteboard) | `springboard/apps/clock-desktop/src/clock/mod.rs` (Copy Laps), `springboard/apps/clock-desktop/src/main.rs`, `springboard/sdk/src/pasteboard.rs` |
| core-transferable | Copy and paste of typed items | `springboard/apps/clock-desktop/src/clock/stopwatch.rs` (`export_text`), `springboard/sdk/src/pasteboard.rs` |

Rows this round does not move (still open): `user-notifications.json` "Notification triggers" (alarms through
`notification-schedule` is not done, see above) and `core-foundation.json` "Locale and time zone (CFLocale, CFTimeZone,
CFCalendar)" (the world clock uses its own city dataset).

## Remaining gaps

- Alarm banners depend on the shell's `slot_of` lookup of the Clock id (shell side, see above).
- Snooze from a notification (shell side).
- No time zone database in the SDK; the world clock uses the Clock's city table.
- The Copy Laps button is in the toolbar only; there is no Edit menu item for it yet.
