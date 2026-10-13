# Alarm and reminder service design (S7)

Status (2026-10-13): the `alarmd` service is still design only; there is no `alarmd` binary, registry, socket or `alarms`
permission in the repo. Since this was written the shell-owned path changed (see "Changes since 2026-10-08" below). Line
numbers in section 1 are from 2026-10-08 and have drifted; search by name. Effort M-L; this doc fixes the scope and the
first step.

## Changes since 2026-10-08

- Gap 1 (sound) is closed: `Effect::Alarm` in `springboard/src/bin/shell.rs` sends `AudioJob::Sound(Sound::Alert(index))` for
  the alert tone (`Shell::alert_tone`), or logs `alarm (muted)` when the tone is off. The alert's `vibrate` and `ramp` fields
  are still unused.
- Fired alarms file their notification under the Clock manifest id `is.olibuijr.clock.desktop` (`clock::MANIFEST_ID`), found
  whether or not the app runs. Before (Clock 1.5.0) the lookup used the data id `is.olibuijr.clock`, which matches no
  installed app, and the banners were dropped.
- The shell re-reads `clock.ini` and `clock-timers.ini` once a second only while the Clock app runs (it is their only
  writer) and every 60 s otherwise (`POLL_MS`, `IDLE_POLL_MS` in `clock/mod.rs`, shell 0.7.9): the one-second poll was the
  shell's main idle wake-up. A scheduler service would replace the poll, not add one.
- Gaps 2 to 6 are unchanged: RTC wake (`rtc_wake`, `/sys/class/rtc/rtc0/wakealarm`) is unverified on Midget, the mechanism is
  Clock-only, Reminders and Calendar alerts have no scheduler, and the only app-to-shell channel is `ToShell::Notify`.

## 1. What exists today (verified 2026-10-08 against the code)

The review (`docs/pc-app-review-2026-10-08.md:17`) says "Clock background
alarms and closed-app reminder/calendar scheduling are not implemented". That
is only partly right. Re-reading the code:

- **Alarms and timers already fire with the Clock app closed**, as long as the
  shell process runs. The service lives in the shell, not in the app:
  - `springboard/src/shell/clock/mod.rs:1-10` (module doc) says so explicitly.
  - `shell/mod.rs:755` `set_data_dir` loads it from
    `appdata/is.olibuijr.clock/clock.ini` (`shell/mod.rs:773-776`).
  - `shell/mod.rs:2220` calls `self.clock.tick(..)` on every shell tick.
  - `clock/mod.rs:316` `tick` re-reads the files when their stamp changes
    (`POLL_MS = 1000`, `clock/mod.rs:33`), fires due alarms
    (`alarm.rs:271` `Alarms::fire`, `GRACE_SECS = 600` at `alarm.rs:9`) and
    timers (`timer.rs:131` `Timers::poll`).
- **The app and the service share files, not messages.** The package writes
  `clock.ini` and `clock-timers.ini` beside its data; the shell re-reads them
  (`clock/mod.rs:151-157`, `160-168`). There is no IPC for scheduling.
- **Writes are atomic**: `clock/store.rs:8` (`write_atomic`, delegates to
  `shell::fsutil`).

So the real gaps are narrower than "not implemented":

1. **No sound on a background alarm.** `clock/mod.rs:75-99` says the shell
   "plays it on `Effect::Alarm`", but `bin/shell.rs:777` only logs
   `springboard: alarm`. The alert's `sound`, `vibrate` and `ramp` fields are
   carried and never used. (Sound is also gated by the board rule: no sound
   without the user asking at that moment; see AGENTS.md and bifrost-audio.)
2. **Wake from suspend depends on RTC.** `Effect::WakeAt` goes to `rtc_wake`
   (`bin/shell.rs:1298`), which writes `/sys/class/rtc/rtc0/wakealarm`. On the
   PC (Midget) this path is unverified; failures are only logged
   (`bin/shell.rs:1303`).
3. **The mechanism is Clock-only.** `APP_ID` is a constant
   (`clock/mod.rs:30`) and no other package can schedule anything.
4. **Reminders have no scheduler.** `is.olibuijr.reminders` stores
   `reminders.tsv` in its appdata (`shell/apphost.rs:806`), nothing reads it
   for timing. The Calendar store has an `alert: u8` index into `ALERTS`
   (`shell/calendar/store.rs:28,39`) that nothing fires.
5. **The only existing app-to-shell channel is `ToShell::Notify`**
   (`springboard/proto/src/lib.rs:86`), a plain banner with no time.
   Notifications are rate limited per app (review, line 10).
6. **The app socket protocol** (`shell/apphost.rs:1-3`, `AppHost::spawn` at
   `apphost.rs:206`) is per running app over a `SOCK_SEQPACKET` socketpair; it
   cannot reach an app that is not running.

## 2. Goals and non-goals

Goals:
- Alarms, timers and reminders for any installed package fire while that
  package is closed, and while the shell is restarting.
- One owner per timer, so an alert never rings twice.
- Small surface: a file format, one socket, a handful of messages.

Non-goals (this design):
- Calendar recurrence, time zones, invitations. Calendar stays a UI over its
  own file until a later step.
- Playing sound. The shell plays the alert tone; the service only exposes the sound name.
- Waking a suspended PC. RTC wake is a separate work item.

## 3. Proposed service: `alarmd`

A small system service, `is.olibuijr.alarmd`, that owns all scheduled items.

- Packaging: a `kind = "system"` package with an empty `exec` is never
  spawned by the launcher (`apphost.rs:39`, `:73`). The service needs its own
  supervision: the shell starts it at boot (`boot.sh` / desktop session), or it
  runs as a child of the shell with a restart on exit. Decide which in step 2.
- Ownership: the service is the only writer of the registry (below). Apps
  never write it directly.
- Single process per device. It is a headless binary that reuses
  `shell/clock/alarm.rs` and `timer.rs` (pure logic, already unit tested) by
  moving them into a shared library module. It must not depend on the
  renderer.

### 3.1 Registry (persistence)

One file, `appdata/is.olibuijr.alarmd/registry.ini` under the data dir, written with the
same atomic helper. Key/value style like `clock.ini`:

```
item.0.owner=is.olibuijr.clock
item.0.kind=alarm            # alarm | timer | reminder
item.0.id=1
item.0.at=1760000000         # absolute unix seconds (timers: absolute end)
item.0.repeat=0              # weekday mask, alarms only
item.0.label=Wake up
item.0.sound=Radar
item.0.fired=0               # unix time of the last ring, for idempotence
```

- `fired` makes a restart idempotent: an item that already rang for `at` is not
  rung again. Missed items within `GRACE_SECS` (600 s) still ring, as today.
- Unknown keys are kept on rewrite; junk lines are dropped (same rule as
  `Alarms::parse`).
- The existing `clock.ini` / `clock-timers.ini` are migrated once into the
  registry with `owner=is.olibuijr.clock`. During migration the shell keeps
  reading the old files and does not tick them (see 4).

### 3.2 IPC

A `SOCK_SEQPACKET` unix socket owned by the service, path
`/run/alarmd.sock` (or the per-session equivalent on the PC). Messages are
small and text-framed (one `key=value` line per field, like the registry), so
no new serialisation crate:

| Request | Reply |
|---|---|
| `schedule kind=alarm at=... repeat=... label=... sound=...` | `ok id=N` or `err reason` |
| `cancel id=N` | `ok` |
| `list` | one `item` line per owned item |
| `snooze id=N secs=540` | `ok` |

Rules:
- The owner is never taken from the message. The service reads the peer's
  credentials (`SO_PEERCRED`) and maps the PID to the installed package id via
  the app registry. An app can only touch its own items.
- Requests need the manifest permission `alarms` (new). Packages without it get
  `err denied`.
- The service never sends the ring to an app. It emits an event (below) and
  writes the item's `fired` stamp before it does anything else.

Events (service to shell, on a second socket or the same one, subscription
set by the shell):
- `ring id=N owner=... title=... time=... sound=... vibrate=... snooze=...`
- The shell turns `ring` into the full-screen alert and a notification. If no
  shell is connected, the service appends a banner entry to
  `notifications.ini` (the shell reads it at `set_data_dir`,
  `shell/mod.rs:757`) so the user sees it on the next start.

### 3.3 Timing

- One `timerfd` (or sleep until the next `at`) with `CLOCK_REALTIME`
  wall-clock; re-arm on any registry change and on `CLOCK_REALTIME` jumps
  (the alarm module already re-arms on manual clock changes).
- Wall-clock logic stays in `alarm.rs` (local civil time, DST-safe). The
  service takes the zone offset from the shell's `set_wall`/`set_zone` values
  it is sent, or from `/etc/localtime` when the shell is down.
- RTC wake (`rtc_wake`) stays a separate, optional step: the service asks for
  an RTC alarm for the next item when it can.

## 4. Shell handoff

Single owner at any time. The handoff has three states:

1. **Today (no service):** the shell owns `clock.ini` and `clock-timers.ini`
   and ticks them (`clock/mod.rs:316`). Unchanged.
2. **Shadow:** the service starts, migrates the files once, and mirrors the
   registry. The shell still ticks; the service does not ring. The shell logs
   a mismatch if its due list differs from the service's.
3. **Owned:** the service rings. The shell stops calling `Alarms::fire` and
   `Timers::poll` and only receives `ring` events. `Clock::load` reads the
   registry instead of the two files. `Clock` keeps the UI: alarm cards, the
   editor, the Stopwatch and Timer tabs.

The shell's `Effect::Alarm` path (`bin/shell.rs:777`) becomes the sound hook
for `ring` events. The sound already plays for the alert tone.

If the service dies, the shell restarts it. If the service is down at
suspend, the shell refuses to hand off a timer past the next wake; it keeps
ticking itself until the service is back (state 2 falls back to 1).

## 5. Reminders and Calendar

Reminders and Calendar become schedulers, not timers:
- A reminder with a due time calls `schedule kind=reminder`; the service rings
  it like an alarm with the reminder title. Reminder lists and flags stay in
  `reminders.tsv`.
- Calendar events with `alert != 0` schedule `kind=reminder` at
  `start - ALERTS[alert]`. Recurrence is expanded by the app before scheduling,
  one item at a time (no RRULE in the service).
- Both need the `alarms` permission in their manifests.

## 6. Plan

Step 1 (smallest useful change, next task):
- Move the pure parts of `shell/clock/alarm.rs` and `timer.rs` that the shell
  needs into a shared module (no renderer, no `Ui`), keeping their unit tests.
- Add a headless `alarmd --check <dir>` that loads `clock.ini` and
  `clock-timers.ini` and prints what would fire in the next hour. No socket, no
  ownership change. This proves the shared module and gives a diff tool for
  the shadow state.

Step 2: `alarmd` binary with the registry, the socket and `schedule`/`cancel`/
`list`, plus the shadow mode. Host tests only; no device run without the user.

Step 3: Owned mode, `alarms` permission, migration, Reminders and Calendar
scheduling. Sound and RTC wake need their own approvals (audio rule; PC wake
test on Midget with the user).

Step 4: device checks on Midget with the app closed and the shell
restarted; list goes into the Friday diary entry.

## 7. Tests (planned)

- Host unit tests: registry parse/serialise round trip, junk tolerance,
  `fired` idempotence across restart, missed-within-grace vs. expired, DST and
  clock-jump re-arm (reuse `alarm.rs` tests).
- IPC: `schedule` from an owner is accepted; from another package denied;
  missing `alarms` permission denied; `cancel` of another owner's id denied.
- Handoff: shadow mode produces no duplicate `ring`; owned mode never calls
  `Alarms::fire` in the shell (assert on the handoff flag).
- No cargo run in this task; tests run in the build phase per the workflow.

## 8. Risks and open questions

- **Process model:** is the shell the only long-lived process?
  `boot.sh` restarts it on exit code 3. A separate service changes the boot
  path; `bifrost-boot` must be consulted before step 2. Do not restart adbd.
- **PC session:** the desktop shell is the same binary (`bin/shell.rs`,
  profile `is_desktop()`), so the shell-owned path works there today. Whether
  the PC has `/sys/class/rtc` is unverified.
- **Peer credentials:** `SO_PEERCRED` to package id needs the installed
  registry to map PIDs to packages; confirm that spawned apps are the only
  clients.
- **Suspend:** without RTC wake, a suspended device rings only when it wakes
  for another reason. This is the largest user-visible gap and needs a device
  test.
- **Permission model:** `alarms` is new in the manifest. Store review copy
  (see the review, line 17) must change when it ships.
- Open: should `snooze` belong to the service or the shell? Proposed: the
  service, so a snooze works while the shell is down.

## 9. Corrections to earlier claims

- "Alarms are tied to the shell": true for the logic, but it works with the
  Clock app closed. The real gap is the missing sound, the RTC/PC wake path,
  and the lack of a service for other packages.
- `clock/mod.rs:75-99` said the shell "plays" the sound on `Effect::Alarm`; on 2026-10-08 it did
  not (`bin/shell.rs:777` only logged). It does now.
- `clock/mod.rs:6` says the service reloads the app's files. It does: on a
  stamp change, once a second (`clock/mod.rs:316-327`).
