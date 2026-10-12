# Connect: Remote app to the new system services

Branch `connect/remote`. The Remote app (`springboard/apps/remote/`, version unchanged at 0.2.0) now uses three shell
services that the desktop gained this round: `network` (path changes), `screens` (the desktop size) and `pasteboard`
(shared clipboard text). Remote's sharing service and the shell are unchanged. The new glue is in
`springboard/apps/remote/src/services.rs`; the window logic is in `src/main.rs`.

## What is connected

| Service | Matrix slug | Feature | Where in Remote |
|---|---|---|---|
| `network` (`network.path`, `pathUpdate`) | `network` | Network path monitoring | `main.rs` `service_reply` and `service_event` set the path; the Network line reads it. |
| `screens` (`screens.list`, `changed`) | `appkit-user-interface` | Windows, Panels, and Screens (`NSScreen` size) | The intro line under the title shows the main screen size: "Share this 1440 x 900 desktop in your browser." |
| `pasteboard` (`general` board, `write`) | `application-services` (Pasteboard) | Copy of typed text to the shared clipboard | Tapping an address copies it; the notice line under the sharing controls says "Copied to the clipboard." or the error. |

Reads of the path are event driven. Remote asks `network.path` and `screens.list` once, at start (first configure).
After that the shell pushes `pathUpdate` and `changed`. A copy is one `pasteboard` write on a tap, nothing else.

Fallback: an older shell answers "no service network". Then the window keeps reading the link with `ip -j` every 5 s,
as it did before. A screen-list error leaves the plain intro line. A copy error shows "Could not copy: ...".

## How a user sees it

- Title area: "Share this <width> x <height> desktop in your browser." once the screens reply arrives.
- Network line: unchanged words ("Connected by Ethernet", "No network connection", ...), now updated when the shell
  pushes a change rather than up to 5 s later.
- Tap the browser address (the TINT line under "Browser address"), or the line "Address: http://..." under the list
  for the selected computer. The notice line at the bottom of the sharing area then says "Copied to the clipboard."
  Paste it into another app (for example a browser address bar) to open the desktop.

## How to verify on the test PC (Midget)

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`, then
   `bifrost pc midget open-app is.olibuijr.remote.desktop`.
2. The intro line shows the panel size in points: a 2880x1800 panel at scale 2 reads "Share this 1440 x 900 desktop..."
   (check `bifrost pc midget state` for the scale).
3. Unplug the Ethernet cable (Midget uses wired VLAN 20). The Network line changes to "No network connection" within
   about a second, not after 5 s (the shell pushes `pathUpdate`). Plug it back in.
4. Tap the address line. The notice line shows "Copied to the clipboard." Open a text field in another app and paste:
   the address must appear.
5. Host tests (for the lead to run once, per the no-build rule of this round): `cargo test -p bifrost-remote`. The new
   tests are in `services::tests` (service calls sent, copy write shape, notice text, path reply, intro text).

## REA evidence

Not gathered in this round. The behaviour follows the existing matrix rows and the parity notes
`docs/parity/wire-svc-network.md`, `wire-svc-screens.md` and `wire-svc-pasteboard.md`, which cite the NWPathMonitor,
NSScreen and NSPasteboard evidence.

## Gaps

- **Clipboard read:** Remote does not read the shared clipboard. The `changed` events from `pasteboard` are not
  subscribed to, because nothing in the window shows clipboard contents yet.
- **Refresh rate and several screens:** the screen list has one entry on PC, and refresh rates are not reported.
  The intro line names the main screen only.
- **Reachability and link quality:** the path says whether there is a route, not whether a computer answers or how good
  the link is (no per-host probe, no constrained flag). Those stay gaps in `wire-svc-network.md`.
- **Advertising:** Remote still does not announce itself on the network (see `wire-app-remote.md`).
- **Selection by index:** a selected nearby computer can point at another row after the list changes (existing gap).
- **Snapshot golden:** `tests/snapshots/remote-{light,dark}.png` no longer match the new text. The lead re-blesses them
  with `BIFROST_BLESS=1` after building.
- **Not built here:** the NO-build rule applies to this round; the host tests and the Midget run are still to do.
