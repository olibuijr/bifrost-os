# Changelog

## 0.2.0 — 2026-10-17

- New app window and manifest (`is.olibuijr.locate.desktop`, window full-size): a sidebar with People, Devices, Items and
  Me; the Devices list shows this machine first, with its battery and whether a position is known; an empty Devices list
  says why.
- First-run sheet explains what is shared (position and battery level) and is dismissed with Return; the dismissal is
  kept in the app's data folder.
- Me page: the Share My Location state and the location state, which reads "Location services unavailable" until the
  shell has a location service.
- Control methods `locate.locate`, `locate.play_sound` and `locate.share` (`{"on": bool}`). Locate and play sound are
  queued; the cloud client does not send them yet.
- `panel` (window model) and `control` (method parsing) modules with tests.
- SDK: `bifrost_app::cloud_findmy` builds and reads the device list, device command and device report requests of the
  BifrOSt Cloud API (`/findmy/...`).

## 0.1.0 — 2026-10-14

- New crate `bifrost-locate`: the model behind the device and people locator, with no window yet.
- Devices: position fixes that keep the newest one, battery level clamped to 100 percent, lost mode with a message and
  a contact number, and a command queue (Play Sound, Lock, Erase) that de-duplicates requests and needs confirmation
  before Erase is queued.
- Location sharing: grants for an hour, a day or indefinitely, replaced by a later grant to the same address and
  checked for a valid address form.
- Friends: distance from a place, arrival and departure rules that fire on a change of inside state, and distance text.
- Links: `locate:device/<id>` and `locate:person/<address>`, and driving directions through the maps app's link format.
