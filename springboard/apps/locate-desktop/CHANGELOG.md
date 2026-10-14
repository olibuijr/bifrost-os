# Changelog

## 0.1.0 — 2026-10-14

- New crate `bifrost-locate`: the model behind the device and people locator, with no window yet.
- Devices: position fixes that keep the newest one, battery level clamped to 100 percent, lost mode with a message and
  a contact number, and a command queue (Play Sound, Lock, Erase) that de-duplicates requests and needs confirmation
  before Erase is queued.
- Location sharing: grants for an hour, a day or indefinitely, replaced by a later grant to the same address and
  checked for a valid address form.
- Friends: distance from a place, arrival and departure rules that fire on a change of inside state, and distance text.
- Links: `locate:device/<id>` and `locate:person/<address>`, and driving directions through the maps app's link format.
