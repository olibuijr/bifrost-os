# Screen Saver, wave 5: idle monitor and sleep assertions

Branch `parity/w5-screen-saver`, on top of `parity/integration` (waves 1 to 4).

## What was built

Two pure models in `springboard/core/src/`, both std only, with host unit tests. Neither is wired into the shell,
the SDK or any app yet, and neither has been built or run on the VM or Midget.

- `idle_monitor.rs`: `IdleMonitor` tracks the time of the last input against a timeout. `poll` reports
  `BecameIdle` once when the gap reaches the timeout, `note_activity` reports `BecameActive` when input ends an
  idle period. A zero timeout disables idle detection. A clock behind the last input counts as no time passing.
  This is the app-side model of the inactivity that starts a screen saver.
- `sleep_assertions.rs`: `SleepAssertions` is the registry behind an app holding an activity. `begin` returns an
  id and keeps the reason, `end` releases it, and `blocks_display_sleep` and `blocks_system_sleep` answer whether
  a held assertion keeps the screen or the machine awake. `ActivityOptions` carries the two idle-sleep flags plus
  three non-blocking flags. `sleep_reasons` lists the blockers for the shell to show.

Still missing: no SDK wrapper, so an app cannot reach these models through `bifrost_app` yet. The system layer
still reads blockers only from `systemd-inhibit` in `springboard/src/bare/sleep.rs`, and nothing feeds
`SleepAssertions` into that path. Idle state is not exposed to apps. These rows stay `partial` until the SDK
wrapper and the shell wiring exist and run on Midget.

## Official API names mirrored

Screen Saver framework (https://developer.apple.com/documentation/screensaver): the idle-to-saver start is the
framework's inactivity trigger, described on the "How the system runs your screen saver" section.

ProcessInfo.ActivityOptions (https://developer.apple.com/documentation/foundation/processinfo/activityoptions):
`idleDisplaySleepDisabled`, `idleSystemSleepDisabled`, `userInitiated`, `latencyCritical`, `background`, with
`beginActivity`/`endActivity` as the begin and end pair.

REA evidence (rendered official pages, `tools/parity/rea-page.sh`):

- `ev_5f3cf45c762650c0f8c905ee847af78b40bc8fbf3b98ff5706ca513f3528b185` (Screen Saver)
- `ev_38b6561204c39e27a5c92ba51dbd8bd9ef78328cf5e7950ecb75b145b54f9b52` (ProcessInfo.ActivityOptions)

## Matrix rows closed

| Framework | Feature (as in data/screen-saver.json) | New status | BifrOSt path |
|---|---|---|---|
| Screen Saver | Idle detection that starts the saver | partial | springboard/core/src/idle_monitor.rs |
| Screen Saver | Sleep inhibitors held by apps | partial | springboard/core/src/sleep_assertions.rs |

Tests: `cargo` was not run, per the wave rules. The tests in both modules were worked out by hand.
