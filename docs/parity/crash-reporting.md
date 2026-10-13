# Crash reporting and session restore (round 75, slice crash-reporting)

Evidence used: the shell code in `springboard/src/shell/apphost.rs` (`pump`, `reap`, `ext_tick`, `ext_open`),
`springboard/src/shell/session.rs` (session restore), `springboard/src/shell/diag.rs` (crash.log reader),
`springboard/src/bare/crashlog.rs` (crash.log writer), `springboard/apps/console-desktop` (Crash Reports from
`coredumpctl`). The reference macOS 27 files for this slice were not in `~/Projects/macos-ref/extract/` when the
round started, so the behaviour follows the existing BifrOSt designs and the owner's brief; no REA evidence was taken.

## What is done

- An app process that ends on its own with a failure (a signal such as SIGSEGV or SIGABRT, or a non-zero exit) is
  logged to `crash.log` as `app <id> quit unexpectedly: signal 11 (SIGSEGV)`. Its window closes, the shell keeps
  running, and the "quit unexpectedly" alert appears (`springboard/src/shell/crash_alert.rs`).
- The alert has three buttons: Close, Report (saves the report to `<data dir>/last_crash_report.txt` and opens Console),
  and Reopen (opens the app again through `Shell::open`).
- The report quotes this app's newest crash.log lines (at most 8, oldest first).
- Not a crash: a quit the shell asked for (those hosts leave `ext` before they exit) and an app that exits with 0.
- Session restore (already in place, unchanged): `session.txt` is written every 2 s while the desktop runs and at
  restart or shutdown; the next start replays it (apps, frames, maximized and minimized state).

## Matrix rows touched (not edited here; lead closes them)

- `crashreportextension.json`, "Crash record written by the shell and the runtime": other apps' failures now reach
  crash.log through `app_crashed`. Partial: the record exists, but no app SDK call writes it.
- `crashreportextension.json`, "Crash reporter extension point": not closed. The shell does not call an app's
  reporter; the alert is the shell's own report.

## Still needs checking on Midget

- A real segfault in an installed app (for example a test app that calls `abort()`): the alert appears, the shell
  stays up, and Reopen starts a new process.
- Report opens Console; Console's Crash Reports list shows the coredump. Console does not yet open on a view
  chosen by the shell (no launch argument), so the person selects Crash Reports once.
- The alert is drawn on top of the desktop and takes touches until one of its buttons answers.
