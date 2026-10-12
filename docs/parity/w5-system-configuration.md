# Wave 5: System Configuration parity

Status: pure models, host unit tests only. Not wired into the shell or apps, and not yet tested on the VM or Midget.

## What was built

- `springboard/core/src/prefs_session.rs` (`bifrost_core::prefs_session`): a system-wide preferences session. Edits are
  staged in a working copy, commit requires the exclusive lock and returns the changed keys, and apply returns the keys
  to push to the running system once. Revert drops uncommitted edits. Keys are validated (dotted segments, 128 bytes),
  text values are capped at 4096 bytes.
- `springboard/sdk/src/system_prefs.rs`: app-facing re-export of the session types.
- `springboard/sdk/src/wifi_status.rs`: app-facing Wi-Fi status snapshot. It maps the system layer's state code
  (0 off, 1 starting, 2 searching, 3 connecting, 4 connected) to `WifiState`, derives signal bars from dBm, and looks
  up saved networks and problem notes. It makes no service call; netd's `Status` fields are passed in by the caller.

## Official API names mirrored

- SCPreferences (https://developer.apple.com/documentation/systemconfiguration/scpreferences): SCPreferencesCreate,
  SCPreferencesGetValue, SCPreferencesSetValue, SCPreferencesRemoveValue, SCPreferencesLock, SCPreferencesCommitChanges,
  SCPreferencesApplyChanges. Not mirrored: SCPreferencesSetCallback (no callback; the caller reads apply's list).
- System Configuration landing page, which lists the Wi-Fi status and reachability topics:
  https://developer.apple.com/documentation/systemconfiguration

## REA evidence

- `ev_b7ea80711f77cbe3762a3f4996d097fc6a35f5dcb98a7e3d42fd3b751fb427f4`: System Configuration landing page (rendered).
- DocC: `systemconfiguration/scpreferences` (symbol abstract: handle to an open preferences session).

## Matrix rows closed

| framework | feature (as in data/system-configuration.json) | new status | BifrOSt path |
|---|---|---|---|
| System Configuration | Preferences session for system configuration | partial | springboard/core/src/prefs_session.rs |
| System Configuration | Wi-Fi status, scan, join and saved networks | partial | springboard/sdk/src/wifi_status.rs |

Wi-Fi scan, join and the live netd client remain open. The row stays partial because only the status snapshot is
built here.

## Not built

- Console user (gap): no system session accessor exists in the system layer, so there is no capability to wrap.
- Ethernet bond and VLAN (gap): server-class, out of scope for the desktop.
- Network change notifications and interface enumeration (os): net_path.rs and sdk network_path already cover the
  interface model; the live feed still needs a netd client, which this wave does not add.
