# Wire 6: Console app (hang diagnostics, daily summary)

Branch `wire6/console`. App `springboard/apps/console-desktop` (Console 1.3.0 becomes 1.4.0). Only that folder changed,
plus this note. Not built: the lead builds once (`cargo test -p bifrost-console` from `springboard/`).

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | Status after this wire |
|---|---|---|---|
| `metrickit` | Hang diagnostics and UI stall detection (HangDiagnostic, HangTimeMetric) | `springboard/apps/console-desktop/src/diagnostics.rs` (`stall_seconds`, `hang_rows`, `HangSource`), reads `springboard/src/shell/diag.rs` (`load`, `CrashEntry`) | Partial: Reports > Hang Diagnostics lists each UI-loop stall the shell wrote to crash.log, with its time and length. No call stack and no app-facing delivery. |
| `metrickit` | Crash diagnostics (CrashDiagnostic, call stacks) | `springboard/apps/console-desktop/src/source.rs` (`Coredumps`, unchanged), counted by `diagnostics.rs` (`DailySource`) | Partial, unchanged in kind: the call stacks are still the `coredumpctl info` text in the detail pane. The new Daily Summary counts these rows per day. |
| `metrickit` | Immediate diagnostic reports (DiagnosticReport) | `springboard/apps/console-desktop/src/diagnostics.rs` (`HangSource`, `DailySource`) | Partial: the shell's crash and hang records are shown in the Console, one per row. Nothing is delivered to an app. |
| `metrickit` | Daily metric reports to the app (MXMetricManager, MetricReport) | `springboard/apps/console-desktop/src/diagnostics.rs` (`DailySource`) | Not closed. The Daily Summary counts crashes and hangs per UTC day. It is not the app-facing report: the shell's per-app counters (`bifrost_core::app_metrics::Totals`) are not sent to the Console, so no metric payload is shown. The model in `springboard/sdk/src/daily_metric_report.rs` is still not fed. |

Pure code reused, not rewritten: `springboard::shell::diag::{load, CrashEntry, parse_line, signature}` (the shell's own
crash.log reader), `springboard::bare::platform::data_dir` (the data directories), the `Coredumps` source (the
coredumpctl list). New pure code in the app: `stall_seconds`, `hang_row`, `hang_rows`, `Day`, `by_day`, `summary`,
`daily_rows`, all in `diagnostics.rs`.

The shell writes "ui loop stalled for Ns" in `springboard/src/bare/heartbeat.rs` when its UI loop stops answering for
20 s. The Console reads that line from the crash.log of each data directory (the system one, `/var/lib/springboard`,
and the user one, `~/.local/share/springboard`).

## How a user reaches it

- **Sidebar > Reports > Hang Diagnostics** (warning-triangle glyph). One row per stall, oldest first. Columns: Type
  (Fault), Time, Process (`springboard`), Category (`heartbeat`) and Message (`ui loop stalled for 21s`). The footer
  reads the count. The status note says a hang is the UI loop stopping for at least 20 s and that no call stack is kept.
- **Detail pane of a hang row**: "The shell's UI loop stopped answering for N s." and "Call stack: none", with a pointer
  to Crash Reports.
- **Sidebar > Reports > Daily Summary** (calendar glyph). One row per UTC day that has a crash or a hang. Message:
  `2 crashes, 1 hang, 21 s stalled` (only the parts that happened are named). Type is Fault for a day with a crash and
  Error for a day with only hangs. Time is the day's start (midnight UTC).
- **Sidebar > Reports > Crash Reports** is unchanged.
- The Reports section now lists: Crash Reports, Hang Diagnostics, Daily Summary, Signposts, Security Events.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`). Midget is the
   test machine; the VM is skipped by owner rule.
2. With no stall recorded, `bifrost pc midget open-app is.olibuijr.console.desktop`, then select Hang Diagnostics: the
   footer reads "No messages" and the empty state reads "No Hang Diagnostics".
3. Write a stall the way the heartbeat does (on Midget, as the user the shell runs as):
   `echo "[1760227200 12:00:00] thread 'main': ui loop stalled for 21s" >> ~/.local/share/springboard/crash.log`
   (use `/var/lib/springboard/crash.log` if the shell runs as root). Reopen Hang Diagnostics: one row, "ui loop stalled
   for 21s", and the detail pane says 21 s and "Call stack: none".
4. Select Daily Summary: one row for 2025-10-12 (1760227200 is 2025-10-12 00:00:00 UTC) reading "1 hang, 21 s stalled",
   Type Error. Its time is midnight UTC.
5. Take screenshots with `bifrost pc midget screen /tmp/console-wire6.png` and check the new rows' glyphs against Crash
   Reports. The glyph names `exclamationmark.triangle` and `calendar` were not checked against the kit's glyph table on
   a device; if a row has no glyph, use an existing name from the table (for example `clock`).

## Host tests

Unit tests in `springboard/apps/console-desktop/src/diagnostics.rs` (run with `cargo test -p bifrost-console` from
`springboard/` when the lead builds once):
- `a_stall_message_gives_its_seconds_and_nothing_else_does`: `ui loop stalled for 21s` gives 21; a missing number, a
  missing unit and a panic line give None.
- `hang_rows_keep_only_stalls_oldest_first_with_microsecond_times`: of three crash.log entries, two stalls remain,
  sorted by time (50 s then 200 s), with dates 50 000 000 and 200 000 000 µs, level Fault, process `springboard`.
- `by_day_groups_crashes_and_hangs_by_utc_day`: crashes at 0, 86 399, 86 400 and 3·86 400+5, hangs of 21 s at 100 and
  4 s at 86 500, give three days: (0: 2 crashes, 1 hang, 21 s), (1: 1 crash, 1 hang, 4 s), (3: 1 crash).
- `a_time_before_the_epoch_falls_on_the_day_before`: time −1 is day −1.
- `the_summary_names_only_the_parts_that_happened`: "2 crashes, 1 hang, 21 s stalled", "1 crash", "2 hangs, 40 s stalled".
- `daily_rows_start_at_midnight_utc_and_take_their_level_from_the_crashes`: a crash day at 86 400 s has date 86 400 000 000
  µs, Fault, "1 crash, 1 hang, 4 s stalled"; a hang-only day is Error.

No window test reads the real crash.log: the sources check `testing::pinned()` and return nothing under the testing kit,
and the window test would otherwise depend on the machine's files.

## Lead notes

- `tests/compliance.rs` snapshots the window. The Reports section gains two rows (Hang Diagnostics, Daily Summary), so
  the sidebar part of the dark and light golden images changes. Bless the snapshot after the build.
- The Log menu, the Edit menu and the window's menus are unchanged. No menu item is added.
- The window background stays `CONTENT_BG` (already in `draw.rs`). No `DETAIL_BG` appears in this app.
- `springboard/Cargo.lock` pins `bifrost-console` at 1.3.0. Building without `--locked` updates it to 1.4.0; the lock
  update belongs in the release commit.
- Pre-existing, not changed here: the sidebar's "This Mac" label and the comment that names it contain a platform name
  (`Mac`) in code. It predates this wire; the owner rule for names applies to it.

## REA evidence

- `ev_5a3cfa41a70c91eff99faac57a5f29434a1f394f3d543eeda96d4c4f0e0a9ef0`: the rendered CrashDiagnostic page
  (`documentation/metrickit/crashdiagnostic`), read with `tools/parity/rea-page.sh`. It lists `callStackTree` (the
  application call stack tree of the crash), the exception fields, and the termination category and reason.
- `ev_565127097c42ceb11f645dadcd0a33856ce72356eef3a38fdd4e03d1aa71b927`: the MetricKit landing page, already on the
  `metrickit` matrix record.
- `ev_95590bda05b3a70a79843ec60bb4a197e8fbd9a24058f1f8d29541cbb288d1ee`: the CallStackTree page, from wave 4
  (`docs/parity/w4-metrickit.md`).
- `ev_50f9061fc408dfb7824ef34bba9be0944529a0f0d69c011c36a062267479a4e3`: the MXMetricManager page, from wave 4.
- DocC: `python3 -I tools/parity/docc.py metrickit/crashdiagnostic` gives the same structure, and
  `python3 -I tools/parity/docc.py metrickit/diagnosticreport` gives the report's `timeRange`, `environment` and `result`.
  The DiagnosticReport content (`result`, its types) was not read in full; only the names were.

## Remaining gaps

- **Daily metric reports to the app**: the shell's per-app totals are not sent to the Console, and no SDK call returns a
  daily payload to an app. The Daily Summary counts crashes and hangs; it does not show the metric payload.
- **Call stacks for hangs**: the shell records only the stall length. A stack needs sampling in the UI loop, which
  does not exist, so the hang row says "Call stack: none".
- **Call stacks for crashes**: the detail still shows `coredumpctl info` text. The structured CallStackTree model
  (`springboard/sdk/src/call_stack_tree.rs`) is not fed from it, because coredumpctl's text is not a sampled stack list.
- **Log archives (File > Save and Open)**: no log archive model exists. OSLogStore(url:) reads an archive, and
  nothing writes one. The window still opens plain log files through Log Files.
- **Memory and CPU exception diagnostics, hang duration histograms, disk and GPU totals**: no feed exists for them in
  the Console, so they are not shown.
- **Hitch measurement** (`springboard/sdk/src/hitch_ratio.rs`) is not fed by the shell's jank counter
  (`springboard/src/bare/jank.rs`), so no hitch row is shown.
- **Time zone**: the Daily Summary uses UTC days. A local-day view would need `springboard::bare::timezone::offset_at`
  per row.

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| `metrickit` | Hang diagnostics and UI stall detection (HangDiagnostic, HangTimeMetric) | `springboard/apps/console-desktop/src/diagnostics.rs` (`HangSource`, `hang_rows`); `springboard/src/shell/diag.rs` |
| `metrickit` | Crash diagnostics (CrashDiagnostic, call stacks) | `springboard/apps/console-desktop/src/source.rs` (`Coredumps`); `springboard/apps/console-desktop/src/diagnostics.rs` (`DailySource`) |
| `metrickit` | Immediate diagnostic reports (DiagnosticReport) | `springboard/apps/console-desktop/src/diagnostics.rs` (`HangSource`, `DailySource`) |
| `metrickit` | Daily metric reports to the app (MXMetricManager, MetricReport) | not closed; `springboard/apps/console-desktop/src/diagnostics.rs` (`DailySource`) counts crashes and hangs only |
