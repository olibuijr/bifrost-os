# Task 07: socket-activated services, nothing before desktop-ready

Status: implemented and reviewed by reading only. Not built, not run (the lead builds once after all waves are merged).
Contract: `docs/boot-architecture.md`, "Services on demand" and "Budgets".

## What is implemented

1. **bifrostd binds, then activates** (`springboard/bifrostd/src/lib.rs`)
   - Every `kind = "service"` package gets its socket bound by the supervisor at `<root>/run/<id>.sock` on first sight,
     with no child running. Services are the three in `springboard/system/`: `is.olibuijr.audio`, `is.olibuijr.net`,
     `is.olibuijr.update`.
   - A child is spawned only when a connection is waiting on its socket (`poll(2)` with zero timeout in `pending`, and a
     blocking `poll` in `Supervisor::wait`, which bifrostd's main loop calls instead of a fixed sleep). The child gets
     the listening socket as its stdin and `BIFROST_ACTIVATED=1`.
   - A child that exits is restarted on the next waiting connection, with the existing backoff (1 s doubling to 30 s).
     It is never given up on.
   - A new `current` is swapped in on a side socket (`<id>.sock.new`) and moved into place only after it answers a ping.
     The new version's listener is kept by the supervisor, so the socket the running child accepts on is the one clients
     reach.

2. **Services accept the activated socket** (`springboard/proto/src/svc.rs`, `svc::listen`)
   - Under `BIFROST_ACTIVATED=1` the service takes fd 0 as its listener. Otherwise (a service run by hand) it binds
     `BIFROST_SERVICE_SOCK` itself, as before.
   - Used by `springboard/audiod/src/main.rs`, `springboard/netd/src/main.rs` and `springboard/updated/src/main.rs`.
     Without this, a service that removed the socket and rebound it would orphan the supervisor's listener and clients
     would hang.

3. **Lazy audio client** (`springboard/src/svc.rs`, `springboard/src/shell/mod.rs`)
   - `audio::Client::for_root` starts unarmed: `tick` does not connect and `next_wake` schedules nothing, so the shell
     sends nothing that would start `audiod`.
   - `Shell::draw` calls `self.audio.arm()` each frame (idempotent), so the connection is made from the first desktop
     frame on, not on the boot path.
   - The shell has no other service clients: `springboard/src/shell/desktop_network.rs` runs `nmcli` directly, and
     nothing in `springboard/src` connects to the net or update sockets. `springboard/src/shell/services/` has no
     connect paths to change.

4. **Units** (`packaging/arch/`)
   - `bifrostd.service`: runs `/usr/bin/bifrostd /var/lib/springboard`. `WantedBy=bifrost-after-desktop.target`,
     `After`/`PartOf` the target.
   - `bifrost-after-desktop.target` (new): reached once the first desktop frame is on screen.
   - `bifrost-desktop-ready.service` (new): oneshot, `ExecStart=/usr/bin/bifrostd desktop-ready`. It waits up to 6 s for
     the `desktop-ready` mark in `/run/bifrost/boot-marks`, then starts the target with `systemctl start --no-block`.
     `WantedBy=graphical.target`.
   - `bifrost-mcp.service`: moved from `multi-user.target` to `bifrost-after-desktop.target`.
   - `springboard.install` enables the two new units on update; `package-files.sh` and `PKGBUILD` install the binary and
     units (the `PC_BIFROSTD` branch mirrors the existing `PC_MCP` one).

5. **`bifrostd desktop-ready`** (`springboard/bifrostd/src/main.rs`) is the only new mode of the binary. The default
   mode is unchanged apart from the socket activation.

## How it maps to the contract

| Contract line | Where it is met |
|---|---|
| "Every bifrostd service is socket-activated and started on its first message" | `Supervisor::tick` / `pending` / `spawn`; `svc::listen` in audiod, netd, updated |
| "clients connect lazily" | `audio::Client::for_root` is unarmed until `Shell::draw` arms it |
| "units the first frame does not need move after bifrost-after-desktop.target reached on desktop-ready" | `bifrost-desktop-ready.service`, `bifrost-after-desktop.target`, `bifrostd.service`, `bifrost-mcp.service` |
| "Nothing starts before desktop-ready unless the first frame needs it" | Only seatd, springboard and bifrost-privd are ordered before the desktop (see the open items) |

## Tests (host, not run)

Pure logic, in `springboard/bifrostd/src/lib.rs` (`mod tests`):
- `manifest_only_accepts_safe_services` (existing), `backoff` (existing)
- `a_mark_is_matched_by_whole_event_name`: `desktop` is not `desktop-ready`
- `a_socket_is_named_after_its_service_under_run`: `<root>/run/<id>.sock`
- `marks_skip_blank_and_one_word_lines`
- `the_desktop_ready_names_and_wait_are_the_contract`: the mark, the target name, the 6 s wait above the reveal's 4 s
  ceiling, `TICK <= DESKTOP_READY_WAIT`
- `waiting_for_a_mark_returns_at_once_or_at_the_ceiling` (file-based)
- `a_service_starts_only_when_a_client_is_waiting` (filesystem and a `sleep` child, with a real connect)

In `springboard/src/svc.rs`: `a_lazy_client_connects_only_once_armed`.

The lead should run `cargo test -p bifrostd` and `cargo test -p bifrost-proto` and the springboard audio svc tests
once merged.

## Open items (left for the lead or a later wave)

- **`desktop-ready` has no writer yet.** Nothing in `springboard/src` writes the mark (task 09 and task 06 own the
  boot-mark writers). Until one does, `bifrost-desktop-ready.service` waits the full 6 s ceiling on every boot, and that
  delay is on the boot path (the oneshot holds `graphical.target`). Either the shell's reveal writes `desktop-ready`,
  or this unit's wait is shortened.
- **`bifrost-privd.service` is `Before=springboard.service`** and `WantedBy=graphical.target`. It is on the critical
  chain before desktop-ready. Whether the first frame needs it was not verified (its use is in the login round). It was
  left in place. If it is not needed for the first frame, move it to the target the same way as bifrostd.
- **Audio reconnect after bifrostd starts.** The client's `next_wake` schedules a reconnect only while the socket path
  exists, and before bifrostd runs the path does not exist. When bifrostd binds the socket, the shell must wake to
  reconnect; if the shell does not wake on its own, add a periodic wake while the client is armed and disconnected.
  Not verified on Midget.
- **Services are not reachable before the target.** `bifrostd.service` starts after desktop-ready, so the audio, net
  and update sockets do not exist during the first frame. The lazy client copes (it retries); apps that need the net or
  update services at login would need the target earlier. None of the current shell clients need them at first frame.
- **Boot timing budgets** (`Bootweave start to desktop-ready < 2.0 s`) are not measured here; they need the baseline
  from task 01.

## Midget verification (for the lead, after publish and `bifrost pc midget update --restart`)

Run these from Titan, after `bifrost pc midget recovery` reports `phase=accepted`.

1. Units and ordering:
   ```
   ssh midget systemctl is-enabled bifrostd.service bifrost-desktop-ready.service bifrost-mcp.service
   ssh midget systemctl list-dependencies bifrost-after-desktop.target
   ```
   Expect `bifrostd.service` and `bifrost-mcp.service` under the target.

2. Critical chain before the desktop. Reboot Midget first (`bifrost pc midget update --restart`), then:
   ```
   ssh midget systemd-analyze critical-chain bifrost-desktop-ready.service
   ssh midget systemd-analyze critical-chain springboard.service
   ```
   Before desktop-ready, expect only seatd, springboard and the chrome clients. `bifrost-privd` appears until it is
   moved (see open items).

3. Desktop-ready and the target:
   ```
   ssh midget cat /run/bifrost/boot-marks
   ssh midget journalctl -b -u bifrost-desktop-ready.service --no-pager
   ssh midget systemctl is-active bifrost-after-desktop.target bifrostd.service
   ```
   Expect `desktop-ready` in the marks (when a writer exists) and the target active after it. Without a writer, the
   journal shows the "no desktop-ready mark within 6s" line.

4. Sockets exist but no service runs:
   ```
   ssh midget 'ls -l /var/lib/springboard/run/ && ss -xl | grep is.olibuijr'
   ssh midget pgrep -a audiod || echo "audiod not running"
   ```
   Expect the three `*.sock` files and `audiod not running` after a clean boot.

5. Activation on the first message. Connect to the audio socket without sending anything and check that the service
   started:
   ```
   ssh midget python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.connect("/var/lib/springboard/run/is.olibuijr.audio.sock"); import time; time.sleep(1)'
   ssh midget 'pgrep -a audiod; journalctl -b -u bifrostd.service --no-pager | tail -n 5'
   ```
   Expect `audiod` running and a `bifrostd: started is.olibuijr.audio ... on demand` line.

6. Desktop and shell: `bifrost pc midget state` shows the desktop. Open Music and confirm playback works (the audio
   client connects on demand). Then `bifrost pc midget screen /tmp/shot.png` for the frame.

7. Shutdown: `ssh midget journalctl -b -1 -u bifrostd.service --no-pager | tail` (after a reboot) shows the children
   stopped under the supervisor's exit, not killed by systemd.

Record the critical chain and the marks in the task's Midget notes. If step 2 shows anything besides the expected
units, add it to the open items rather than changing the chain in this task.
