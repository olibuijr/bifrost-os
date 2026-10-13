# Task 12: symmetric shutdown

Contract: `docs/boot-architecture.md`, "Symmetric shutdown" and the budget "Shutdown (desktop fade to power-off) under 2.5 s".
Task file: `tasks/boot/12-symmetric-shutdown.md`. Branch `boot/12-symmetric-shutdown`.

## What is implemented

- **Shutdown request (root side).** `bifrost-bootweave shutdown` (`springboard/bootweave/supervisor.rs`) runs as
  `ExecStop=` of `springboard.service`, before systemd stops the session. It writes
  `/run/bifrost/bootweave/shutdown`, then waits at most 1.2 s for `/run/bifrost/user/shutdown-faded`. It always exits 0.
  It does nothing unless `/etc/bifrost/compositor` says `bifrost-wm` and `/usr/bin/bifrost-wm` exists, which is the same
  choice the session script makes. A missing window server returns at once.
- **Fade (window server side).** `springboard/wm/src/drm.rs` (`Drm::shutdown_overlay`) checks for the request file on
  every frame. When it appears the boot frame is drawn over the desktop, with the opacity from
  `boot::shutdown_alpha` (smoothstep over `SHUTDOWN_FADE_MS` = 450 ms, then held at 1.0). Once the fade is done the
  window server writes the faded marker once. The boot frame stays on screen until power-off. The overlay replaces the
  boot's own fade-in while it is active. Its pure functions are in `springboard/wm/src/boot.rs`, separate from
  `splash_alpha`.
- **Stop budget.** `springboard.service` has `TimeoutStopSec=2s`. `bifrost-mcp.service` and `bifrost-privd.service` also
  have `TimeoutStopSec=2s`. The unit's VT 12 parking (`ExecStopPost=... chvt 12`) is removed.
- **Marks.** `shutdown-request` (supervisor), `shutdown-start` and `shutdown-faded` (window server) go to
  `/run/bifrost/boot-marks` through `bootsplash::mark`, and `shutdown-fade-timeout` (supervisor) if the fade never came.

## Decisions

- The faded marker is in `/run/bifrost/user/`, not `/run/bifrost/bootweave/`. The window server runs as the installed
  account (`packaging/arch/wayland/session`), and `/run/bifrost/bootweave` is created by root with mode 0755, so the
  account could not write the marker there. The session creates `/run/bifrost/user` owned by the account, and the
  supervisor (root) reads the marker from there.
- The fade is driven from the render loop, so it runs only while the window server renders. It draws every frame while
  the request exists.

## Mapping to the contract

| Contract line | Where |
|---|---|
| Fade back to the boot frame (ShuttingDown screen once task 02 lands) | `drm.rs` `shutdown_overlay`, `boot.rs` `shutdown_alpha` |
| Stop services under a 2 s budget | `TimeoutStopSec=2s` in the three units |
| Frame stays until power-off | `shutdown_alpha` holds at 1.0; nothing removes the overlay |
| No text, VT 12 parking removed | `springboard.service` (`ExecStopPost` gone), test `the_unit_no_longer_parks_the_console_on_vt_12` |
| Shutdown under 2.5 s | Not measured yet (see below) |

## Not done / left for the lead

- The shutdown overlay is the boot mark frame (`boot::paint`, the shell's loader frame). The shared API has
  `bootweave_frame::Screen::ShuttingDown` for this, but `render` in `springboard/bootweave/frame/src/render.rs` is still
  `todo!("boot-02")` on this branch, and the window server does not depend on the crate yet. When task 02 lands, the
  overlay should draw `Screen::ShuttingDown` from the frame crate instead of `boot::paint`. That is a one-function change
  in `boot_frame_buffer` (`springboard/wm/src/drm.rs`).
- `tools/boot-bench.py` measures the boot only. Shutdown is not measured, so `docs/perf-history.jsonl` has no shutdown
  number.
- The fade assumes the window server renders a frame within the 1.2 s wait. Nothing was run, so this is untested.
- The stop of the session (`cleanup` in `packaging/arch/wayland/session`) is not timed. If the 2 s budget is exceeded
  on Midget, the unit is killed by systemd and the fade is cut short.
- Nothing is built or tested by this agent (owner rule). `cargo test -p bootweave`-style tests are written by hand only.

## Midget verification (lead)

Run on Midget (192.168.20.140), not in the VM. Use the bifrost-wm window server (`cat /etc/bifrost/compositor` prints
`bifrost-wm`); without `bifrost-wm` there is no fade and the test does not apply.

1. Install the build and start a desktop: `bifrost pc midget update --restart`, then `bifrost pc midget key enter`.
   Check `bifrost pc midget state` shows the desktop.
2. Clear old marks: `ssh midget 'sudo truncate -s 0 /run/bifrost/boot-marks; sudo rm -f /run/bifrost/user/shutdown-faded'`.
3. Start a timer and stop the unit, keeping the desktop as the test subject:
   `ssh midget 'T0=$(date +%s%N); sudo systemctl stop springboard.service; echo $(( ($(date +%s%N)-T0)/1000000 )) ms'`.
   The printed value is the stop time seen by the caller. Expected: under 2500 ms, and the frame stays the boot frame.
4. Read the marks: `ssh midget 'cat /run/bifrost/boot-marks; sudo cat /var/log/bifrost-bootweave.log | tail -5'`.
   Expected in order: `shutdown-request`, `shutdown-start`, `shutdown-faded`, with `shutdown-faded` about 450 ms after
   `shutdown-start`. `shutdown-fade-timeout` must not appear.
5. Frame check: `bifrost pc midget screen /tmp/shut.png` right after the stop, and compare it with the loader's frame
   (`bootweave-frame` render). There must be no text, no black frame and no Linux console. Wait 5 s and screen again:
   the frame must be unchanged.
6. Power-off timing: `ssh midget 'sudo systemctl poweroff'` while the screen shows the frame; record the time from step 3
   to the power-off with a phone or a camera if the lab has no serial line. The budget is 2.5 s from the fade start.
7. Restart check: `ssh midget 'sudo systemctl restart springboard.service'` shows the desktop again after the boot frame
   and no leftover `shutdown` marks (the supervisor clears them in `run()`).

Record the results in `docs/perf-history.jsonl` only after step 3 and step 6 are measured.
