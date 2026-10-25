# Changelog

## 0.4.6

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.4.5

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.4.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.4.3

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.4.2

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.4.1 — 2026-10-17

- The BifrOSt Account session comes from the SDK helper `bifrost_app::cloud_account::session` (the `is.bifrost.account`
  group, item `bc_session`), not from its own reads. The session is a JSON item (token, account, server, expiry); a
  session that does not decode counts as signed out. The group is granted only to first-party system apps.

## 0.4.0 — 2026-10-17

- Account key in the keychain: the first device that turns Share My Location on creates the Find My key pair; the
  private key is stored under this app's keychain items and read back at start (`bifrost_locate::keys`).
- Cloud worker (`bifrost_locate::cloud`): one background thread registers this device, long-polls its commands (25 s),
  reads the device list, pending devices, shared positions and the account feed about once a minute, and sends the
  jobs the window queues. Device positions are opened with the account key. The session comes from the shared keychain
  group `is.bifrost.account` (item `bc_session`).
- Location: a report asks the location service (`core_location`, when-in-use), seals the fix with HPKE to the account
  key with the device id, adds the battery level when known, and uploads it. Reports go out at most every 15 minutes,
  and at once on a locate request. The manifest carries the `[privacy] location` purpose string.
- Controls: the Share My Location switch and the join button on the Me page; the approval sheet on any page lists
  devices waiting on the account with Approve and Deny; the People page lists positions shared with the account.
- Play sound: the device plays the Bell alert tone through `ToShell::PlaySystemSound` (a new shell message, with up to
  four sounds queued per frame), then acknowledges the command.
- Account feed events (`command_done`, `share_received`, `share_ended`) post shell notifications.
- Queued locate and play-sound requests from the control methods are sent to the account.
- Not built: copies of positions to contacts (no contact share is created yet), and the join key is held only in memory
  (a restart during a wait begins again). The worker stays off until the account sign-in stores the session.

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
