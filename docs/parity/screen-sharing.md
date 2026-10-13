# Screen Sharing (parity round 75)

Slug `screen-sharing`. Covers the Screen Sharing app (viewer), the Remote app (server), the Settings Sharing toggle,
and the RFB protocol they share. Clean room: the names come from the macOS 27 index and from the existing code, and no
reference files are copied into the repo.

## Evidence used

- Reference index: `~/Projects/macos-ref/extract/ref27-index.tsv`. Entries used:
  `System/Applications/Utilities/Screen Sharing.app/Contents/Info.plist` (3498 bytes),
  `System/Library/CoreServices/RemoteManagement/ScreensharingAgent.bundle/Contents/Info.plist` (868 bytes),
  `System/Library/ExtensionKit/Extensions/Sharing.appex/.../ScreenSharing.nib` (Base.lproj nib).
  The app's Info.plist and the nib were not yet extracted to `ref27/` when this round ran, so no REA evidence id exists
  yet. The viewer's menus and options come from the existing BifrOSt Remote app and the RFB 3.8 standard, not from a
  measured reference. Re-check `inspect_plist` and `decode_interface_builder` once the files land.
- Matrix rows (not edited): `docs/parity-matrix/os/apps/s-s.json` "Screen Sharing" (partial, bifrost `springboard/apps/remote`),
  `docs/parity-matrix/os/frameworks/s-s.json` ScreenSharing, ScreenSharingKit, ScreenSharingServer (private, behaviour only),
  `docs/parity-matrix/os/daemons/a-c.json` com.apple.screensharing, .agent, .MessagesAgent (gap).
- Existing code: `springboard/apps/remote` (WayVNC on a private Unix socket, noVNC in a browser on port 12000),
  `springboard/src/privd/accounts.rs` (`service_unit`: `screen_sharing` maps to `wayvnc.service`),
  `springboard/src/shell/sharing_page.rs` (Settings > Sharing toggles, including Screen Sharing).

## Per row

| Entry | API or behaviour | BifrOSt path | Done | Still needs Midget or code |
|---|---|---|---|---|
| Screen Sharing.app (viewer) | Connect to a VNC/RFB server, view, control, scale, clipboard | `springboard/apps/screen-sharing` (new, `is.olibuijr.screensharing.desktop`), protocol in `springboard/rfb` (`bifrost-rfb`) | Connect by address, RFB 3.3/3.7/3.8, security None, Raw and CopyRect, DesktopSize, fit or actual size, pointer, right-click, keyboard text and control keys, View Only, clipboard both ways | Midget check against a TCP WayVNC server (`tools/midget-vnc-probe.py` shows one answers RFB 3.8 without auth) |
| Screen Sharing.app (server) | Share this desktop | `springboard/apps/remote` (unchanged) | Existing: browser login on port 12000, WayVNC on a private socket | Not a raw TCP VNC endpoint, see gaps |
| Settings > Sharing > Screen Sharing | Turn the server on and off | `springboard/src/shell/sharing_page.rs`, `springboard/src/privd/accounts.rs` | Existing toggle, unchanged | `wayvnc.service` has no unit in the repo; the toggle has nothing to start on TCP (see gaps) |
| ScreenSharing, ScreenSharingKit, ScreenSharingServer (private frameworks) | Behaviour only, no app-facing API | none | not applicable | none |
| com.apple.screensharing daemons and agents | Server launch | the Remote app's supervisor, `springboard/apps/remote/assets/server.py` | partly: the supervisor starts WayVNC on demand | not closed |

## Decisions

- The server stays WayVNC. The brief said to reuse a system VNC server through the service manager when one exists, and
  WayVNC is already packaged (`packaging/arch/runtime-packages.txt`) and driven by the Remote supervisor. No RFB server or
  Tight encoder is written in this round.
- The protocol lives in its own crate, `springboard/rfb`, std-only, with no sockets in the protocol functions. The app
  owns the threads. Tests cover the handshake, pixel formats, message bytes, Raw and CopyRect decoding, and a loopback
  session in `apps/screen-sharing/src/connection.rs`.
- Not implemented: VNC (DES) password authentication (refused with "password (VNC) authentication"), TLS, Tight, ZRLE,
  Hextile, RRE, cursor pseudo-encodings, scrolling in actual-size mode (the desktop is centred and cropped), and a saved
  address book.

## Gaps the lead should decide

1. The Remote server does not listen on TCP. WayVNC runs with `enable_auth=false` on a private Unix socket
   (`apps/remote/assets/server.py`). Settings > Sharing > Screen Sharing calls privd `service.set` for `wayvnc.service`,
   which has no unit in the repo. Making a raw VNC endpoint reachable needs a unit, a listening address, and an
   authentication decision: enabling it without a password would expose the desktop. This round did not add a unit.
2. The viewer needs a password path before it can reach an authenticated server. That means DES for VNC authentication.
3. The app's compliance test (`tests/compliance.rs`) and snapshots are not generated. Run
   `tools/gen-app-compliance-tests.py` for `screen-sharing` and bless the goldens on Midget.
4. The app icon is a copy of the Remote icon. Replace it with a dedicated icon.

## Checks not run

Per the round rules nothing was built or tested here. The lead should run `cargo test -p bifrost-rfb` and
`cargo test -p bifrost-screen-sharing`, and check that the new workspace members in `springboard/Cargo.toml` resolve
and `Cargo.lock` updates.
