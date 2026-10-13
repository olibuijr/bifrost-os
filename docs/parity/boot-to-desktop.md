# Boot to desktop (slice boot-to-desktop)

Goal (owner): the desktop runs smoothly on the lab PC (Intel GPU). This slice makes the path from login to a usable
desktop shorter and keeps slow work off it. Code and commits only; the lead builds and tests on Midget.

## What was read

- `packaging/arch/springboard.service`: starts the shell early (DefaultDependencies=no), `ExecStartPre=seed-data`
  (copies packaged assets that the data dir lacks, on every start), `Wants=seatd.service`, `Conflicts=getty@tty1`.
- `packaging/arch/bifrost-privd.service`: `Before=springboard.service` but no `Requires=`; a dead privd does not stop the
  desktop (the login window shows "Login service unavailable").
- `packaging/arch/bifrost-desktop-ready.service` and `bifrost-after-desktop.target`: `bifrostd desktop-ready` waits up to
  6 s for the `desktop-ready` mark, then starts the after-desktop target without blocking.
- `packaging/arch/bifrostd.service`, `bifrost-mcp.service`: `WantedBy=bifrost-after-desktop.target`, so they start after
  the first frame.
- `packaging/arch/bifrost-audiod.service`: `WantedBy=multi-user.target`, not ordered before the shell.
- `springboard/bifrostd/src/lib.rs`: services are socket-activated (netd, updated and the other service packages start on
  their first connection), so they do not sit on the boot path.
- `springboard/src/shell/apphost.rs` (`ext_tick`), `shell/extras.rs` (`login_items_launch`), `shell/session.rs`
  (`session_restore`), `springboard/src/bare/wayland.rs` (`mark_first_frame`).

## Finding

`ext_tick` ran `login_items_launch` and `session_restore` on the shell's first tick. The desk exists from construction,
so every login item was spawned (`ext_open` -> `AppHost::spawn`, which reads the manifests) and every saved window was
opened before the first desktop frame was committed. Those starts competed with the first frame on the main loop.

## Changes

| Area | Path | What |
|---|---|---|
| flag | `springboard/src/bootsplash.rs` | `note_first_desktop_frame()` and `first_desktop_frame_presented()`: one process-wide flag, not compiled out without the Wayland feature |
| flag | `springboard/src/bare/wayland.rs` | `mark_first_frame` uses the shared flag instead of its private static (one place to ask "is the desktop on screen") |
| gate | `springboard/src/shell/desktop_gate.rs` | `DesktopGate`: opens on the first presented frame, or 6 s after the first tick when no frame is reported (the bare display path has no Wayland commit), then stays open. Unit tests |
| wiring | `springboard/src/shell/apphost.rs` | `ext_tick` runs `login_items_launch` and `session_restore` only once the gate is open. `session_tick` still runs; it returns early until the session is restored, so the saved session is not overwritten before the restore |
| wiring | `springboard/src/shell/mod.rs` | `pub mod desktop_gate;`, one field `desktop_gate` on `Shell`, its default in the constructor |

Behaviour: the first frame, the menu bar and the Dock are drawn with no app starts in the way. Login items and restored
windows follow on the next ticks. A failing login item does not block anything: `ext_open` already closes a failed
window and logs the error.

## Already in place (not changed)

- Services are on demand (bifrostd). Audio, network, update and MCP are not on the desktop's critical path.
- `springboard.service` is ordered only against udev, seatd and the mounts; privd and audiod are not waited for.
- The after-desktop target starts after the `desktop-ready` mark, with a 6 s ceiling.

## Not done and why

- `seed-data` runs as `ExecStartPre` on every start and copies with `cp -an` over the whole asset tree. It blocks the
  shell's start. A stamp-based skip would be faster, but a package update must still refresh renamed assets, and the
  cost has not been measured on Midget. Candidate for a measured change.
- The shell's own start (font, first frame cache, wallpaper decode) is outside this slice.

## Still needs Midget checking

- `tools/boot-bench.py installed --runs 5` with the `desktop-first-frame` and `desktop-ready` marks: the first-frame time
  should not move; login items should appear after `desktop-first-frame`.
- A session with login items and saved windows: both still appear after login.
- The bare display path (no Wayland): login items start at the 6 s ceiling.
