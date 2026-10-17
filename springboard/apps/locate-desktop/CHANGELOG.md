# Changelog

## 0.3.0 — 2026-10-17

- SDK `bifrost_app::cloud_findmy`: request builders and parsers for the account key (`PUT`/`GET findmy/keys`), the
  pending devices (`POST`, `GET`, approve, sealed-key poll, `DELETE`), this device's sealed location (`PUT`/`DELETE
  devices/self/location`), contact copies (`PUT shares/{id}/location`) and the shared-with-me list. `CloudDevice` carries
  `sealed_location` from the device list.
- `sealing`: HPKE seal and open for reports (info = device id), share copies and the account key handed to an enrolling
  device (info = pending id). Coordinates are range-checked on read.
- `pairing`: enrollment state for a device that asks to join, and the owner's approval list. Nothing is approved
  automatically.
- `reporting`: report throttle (15 minutes, or at once on a locate while sharing is on), a single clear when sharing
  goes off, and share copies only for live grants.
- Not wired yet: the poll thread, keychain storage of the account key, the approval sheet, the location service, play
  sound on the device, and notifications.

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
