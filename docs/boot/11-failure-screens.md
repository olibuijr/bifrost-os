# Boot 11: failure screens and pstore

> Status 2026-10-13: the Recovery button still has no target (bootweave/supervisor.rs, TODO(boot-11)); it only writes recovery-requested.

Owner: boot-11. Branch `boot/11-failure-screens`. Contract: `docs/boot-architecture.md` ("Text-free on every path",
"One frame"). Task: `tasks/boot/11-failure-screens.md`.

Status: code written and read-reviewed; NOT built or tested (the owner rule for this wave). The lead builds once after
all waves merge.

## What is implemented

1. **Rollback screen** (`springboard/bootweave/efi/src/screen.rs`, `springboard/bootweave/efi/src/main.rs`).
   When the loader starts the previous generation (the `fallback` path, or the failed-current path with a marker),
   `rollback()` draws `Screen::Rollback` through GOP and then starts the previous line. If there is no previous line,
   nothing is drawn and the old result (`LOAD_ERROR` or the current status) is returned, as before.
   `screen::show` renders with `bootweave_frame::render` at the current GOP mode size, converts the `0x00RRGGBB` frame
   pixels to `BltPixel` (B, G, R byte order), and blits the full panel. A missing GOP returns `false`, and the boot
   continues without a screen.
2. **Start-failure screen** (`springboard/bootweave/supervisor.rs`).
   - `after_exit(count, clean, ran_ms)` is the pure policy: a clean exit ends the service (`Done`); a session that ran
     for `STARTED_MS` (20 s) or more restarts without counting (`Retry`); a quick failure counts, and the third in a row
     shows the screen (`Screen`), which then holds.
   - The count lives in `/run/bifrost/bootweave/failures`, so it survives the service's restarts within a boot. It is
     reset to 0 when the desktop's first frame is seen (`desktop-first-frame`).
   - `start_failed()` draws `Screen::StartFailed { selected }` (Restart is selected first), reads key events from the
     input devices (Left/Right/Tab move without wrapping, Enter/Space activate). Restart runs `systemctl reboot`.
     Recovery is a TODO: it only writes the `recovery-requested` mark. If no display opens, the supervisor parks
     instead of restarting in a loop.
3. **pstore crash notice** (`springboard/src/pstore.rs`, called from `supervisor.rs` `run()`).
   - `crash_records(/sys/fs/pstore)` lists the `dmesg-<backend>-<id>` files (panic and oops dumps only; `console-`,
     `pmsg-`, `ftrace-` are left alone). Names are sorted, which is lexicographic.
   - When any exist, `crash_notice()` draws `Screen::RestartedAfterProblem` once, waits 3 s, then removes the records.
     With no display, the records are kept for the next boot.
   - Marks: `crash-notice-shown`, `start-failed`, `start-failed-shown`, `restart-requested`, `recovery-requested`.
4. Dependency: `bootweave-frame = { path = "bootweave/frame" }` added to `springboard/Cargo.toml` and
   `springboard/bootweave/efi/Cargo.toml` (the frame has no dependencies). The two `Cargo.lock` files change only by that
   path dependency.

Unit tests (hand-checked, not run):
- `pstore.rs`: 6 tests (crash-record names, empty ids, sorting and filtering, empty input, missing directory,
  a temp directory where only the crash record is removed).
- `supervisor.rs`: 7 tests (clean exit, quick-failure count to the screen, long run restarts, spawn failure counts,
  count file parsing, key mapping, button selection without wrap, first button is Restart).
- `efi/src/screen.rs`: 2 tests on `bgrx_to_blt`. The efi crate is a UEFI target (`no_std` with a panic handler), so
  these only run if the crate is ever built with a host test harness. Nobody runs them today.

## Contract mapping

| Contract item | Where |
|---|---|
| Rollback notice during a failed-update fallback | `efi/src/main.rs` `rollback()`, `efi/src/screen.rs` |
| Start failure: Restart and Recovery, no text | `supervisor.rs` `start_failed()`, `Screen::StartFailed` |
| Crash notice from pstore, records removed once shown | `src/pstore.rs`, `supervisor.rs` `crash_notice()` |
| One frame: all screens drawn by `bootweave-frame::render` | `screen.rs`, `supervisor.rs` `present()` |
| `bootweave-frame` stays no_std, no dependencies | unchanged by this task |

## Left / open

- **Frame drawing (other task):** `bootweave/frame/src/render.rs` has no per-screen drawing yet (only the bar and
  the `Bgrx` order are visible in the source). `Screen::Rollback`, `StartFailed` and `RestartedAfterProblem` must be
  drawn by the frame before these screens show content. Until then they show the frame's default output.
- **Recovery (TODO):** the Recovery button has no target; it only writes the mark.
- **Appearance:** both the loader and supervisor use `Appearance::Dark` until the timing record carries the user's
  choice (task 06).
- **Restart on hardware:** `systemctl reboot` is used; no power-off path.
- **Input devices:** the supervisor opens `/dev/input/event0..31` once when the failure screen starts. Hot-plugged
  keyboards are not picked up.
- **Unsafe:** none added in this task. The existing UEFI calls (`boot::*`, GOP `blt`) are the only unsafe-free calls
  through the `uefi` crate.

## Midget verification (for the lead, after the build)

Run only on Midget (192.168.20.140). Recover Midget first with `python3 tools/midget.py recovery`.

1. **Start-failure screen:** on Midget, stop the session so it fails quickly, three times in a row (for example,
   make `/usr/lib/bifrost/session` exit 1 for a test build, or `systemctl stop` and restart the service). Expect the
   service to restart twice, then the "BifrOSt could not start" screen with Restart selected. Press Right, then Enter:
   Recovery should write `recovery-requested` to `/var/log/bifrost-bootweave.log`. Press Left, then Enter:
   Restart should reboot. Check the marks with `grep start-failed /var/log/bifrost-bootweave.log`.
2. **Count reset:** after a session that reaches the desktop, check that `/run/bifrost/bootweave/failures` reads `0`.
3. **Crash notice:** the kernel must have pstore (ramoops) configured on Midget. Trigger a panic with
   `echo c > /proc/sysrq-trigger` (test machine only). After the reboot, the screen shows "Your computer restarted
   because of a problem" for about 3 s, then `ls /sys/fs/pstore` no longer lists `dmesg-*`. Check
   `crash-notice-shown` in the log.
4. **Rollback:** force a failed current generation with a marker (see `efi/src/main.rs` `marker` handling) and confirm
   the rollback screen shows before the previous generation starts.
5. Take a Midget screenshot of each screen (`bifrost pc midget screen PATH`) and check it against the frame
   design in `docs/boot-architecture.md`.
