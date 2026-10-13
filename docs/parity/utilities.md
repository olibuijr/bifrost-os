# Parity: utilities (Activity Monitor, Archive, System Information)

Slice owner: utilities (round 75). Scope: `springboard/apps/activity-desktop`, `springboard/apps/archive-desktop`,
`springboard/apps/sysinfo-desktop`, and the shared readers in `springboard/core/src/sysinfo.rs`.

## Evidence used

- Reference extract present locally: `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Activity Monitor.app`
  and `System Information.app` (listed in `ref27-index.tsv`). No REA measurement (`inspect_plist`, `open_binary`,
  `decode_interface_builder`) was run in this round; the behaviour below comes from the existing code and the
  matrix notes. The REA pass is still open for the inspector's tab layout and the System Information category list.
- Matrix rows (`docs/parity-matrix/os/apps/a-c.json`, `s-s.json`) are stale: they say "no process monitor app yet"
  and "no system information app yet", although both apps ship. Not edited, per the brief.

## Activity Monitor (`apps/activity-desktop`, now 0.2.0)

| Reference feature | BifrOSt path | State |
|---|---|---|
| Process table, sort, search, My/Other/All filters | `src/model.rs`, `src/window.rs` | Done (earlier round) |
| CPU, Memory, Energy, Disk, Network tabs with graphs | `src/draw.rs`, `src/sample.rs` | Done (earlier round) |
| Quit Process (SIGTERM, own user only) | `src/window.rs` `quit_selected` | Done |
| Force Quit Process (SIGKILL, own user only) | `src/window.rs` `force_quit_selected`, toolbar `B_FORCE`, menu `MENU_FORCE` | Done this round, needs Midget check |
| Process inspector (open files, sockets, threads, folder) | `src/inspect.rs` (wired), `src/draw.rs` `draw_inspector`, toolbar `B_INSPECT`, menu `MENU_INSPECT` | Done this round: replaces the graph card while open, refreshed each second, closes when the process exits. Needs Midget check |
| Sample Process (stack samples) | none | Gap. Linux has no unprivileged stack sampler; the thread list shows state and CPU ticks instead |
| Memory pressure | `inspect::read_pressure` exists, not drawn | Gap, small: draw it on the Memory tab |

Behaviour notes: an inspected process that other users own shows "Open files are not readable for this process"
when `/proc/<pid>/fd` is denied. Force Quit and Quit share one ownership check, and this process cannot be quit.

## Archive (`apps/archive-desktop`, unchanged this round)

| Reference feature | BifrOSt path | State |
|---|---|---|
| Expand on double-click from Files | `apps/files` (double-click expands, per `apps/files/CHANGELOG.md`) and `apps/archive-desktop` `Input::Open` | Done already |
| Compress from the Files context menu | `apps/files/src/menus.rs` `MENU_COMPRESS`, `MENU_COMPRESS_TGZ`, `MENU_COMPRESS_TXZ` | Done already (Files slice) |
| Progress and history window | `apps/archive-desktop/src/main.rs` | Done |

Not done, on purpose: the app keeps its own in-process tar reader (`read_tar`, `unpack_tar`, `expand_tar`) for
.tar and .tar.gz. It duplicates `springboard::files::archive`, which uses the system `tar`. The app's reader skips
links, devices and unsafe member paths and reports how many it skipped; the shared path does not report skips. Removing
the copy would change that safety behaviour, so it is left for a reviewed change. Candidate follow-up: move the skip
reporting into the shared module, then delete `archive.rs`'s tar code.

## System Information (`apps/sysinfo-desktop`, now 0.2.0; `core/src/sysinfo.rs`)

| Reference feature | BifrOSt path | State |
|---|---|---|
| Hardware categories (CPU, memory, graphics, PCI, USB, displays, power, storage) | `core/src/sysinfo.rs` readers, `apps/sysinfo-desktop/src/model.rs` | Done (earlier round) |
| Software categories (OS, installed apps) | `core/src/sysinfo.rs` `read_installed_apps`, `apps/sysinfo-desktop/src/model.rs` | Done (earlier round) |
| Network: interface state, MAC, Wi-Fi flag | `read_net` | Done (earlier round) |
| Network: IPv6 addresses (global) | `core/src/sysinfo.rs` `parse_if_inet6`, `read_net` | Done this round, via `/proc/net/if_inet6` |
| Network: default route marker | `core/src/sysinfo.rs` `parse_default_route` | Done this round, via `/proc/net/route` |
| Network: IPv4 addresses, gateway address, DNS | none | Gap. `/proc` does not map IPv4 addresses to devices; needs netlink or the netd status |

## Shared-file edits

- `springboard/core/src/sysinfo.rs`: `Roots` gained `proc_net`; `read_net` takes a second argument; `NetInterface`
  gained `addresses` and `default_route`. Only `Report::collect` calls `read_net`.
- `springboard/Cargo.lock`: version lines for `bifrost-activity` and `bifrost-sysinfo` (0.2.0).

## Not verified

Nothing was built or run (the brief forbids builds). Every new function was written against the signatures it calls.
Midget checks still needed: the inspector card layout at phone and full-size widths, the toolbar with five items at
the narrowest window, Force Quit on a test process, and the Network rows with several IPv6 addresses.
