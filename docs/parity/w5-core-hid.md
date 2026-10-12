# Wave 5: Core HID (device discovery and classification)

Two gap rows of the Core HID framework are now partly built in pure Rust: an app-facing device matching model with
snapshot change reporting, and the usage-to-role and transport tables that classify devices. Both are models. The shell
does not yet feed them, so apps get no device list until the shell's evdev scan is wired in.

## What was built

- `springboard/ui/src/hid_match.rs` (`bifrost_ui::hid_match`): `Usage` (usage page and usage ID, with usage constants for
  pointer, mouse, joystick, gamepad, keyboard, keypad, system control, consumer control, touch screen and touch pad),
  `Role` and `Usage::role()`, `Transport` with `name()` and `from_name()`, `Device` with `has_usage()` and `has_role()`,
  `Criteria` with `matches()`, `matching()` over a criteria list, and `changes()`, which diffs two snapshots by
  `unique_id` into matched and removed devices.
- `springboard/sdk/src/hid_devices.rs` (`bifrost_app::hid_devices`): re-exports the model and adds `DeviceManager`, which
  keeps a criteria list and the last snapshot. `update()` returns the devices that started and stopped matching.
- `pub mod hid_match;` in `springboard/ui/src/lib.rs` and `pub mod hid_devices;` in `springboard/sdk/src/lib.rs`.

## Official API names mirrored

Core HID: `HIDDeviceManager` (`init()`, `monitorNotifications(matchingCriteria:)`, `Notification`),
`HIDDeviceManager.DeviceMatchingCriteria` with its initializer fields `primaryUsage`, `deviceUsages`, `vendorID`,
`productID`, `transport`, `product`, `manufacturer`, `serialNumber`, `uniqueID`, `locationID`, `versionNumber` and
`isBuiltIn`, and `HIDDeviceTransport` (the transport cases, with `unknown(_:)` as `Transport::Unknown`).
`HIDUsage` is modelled as a page and usage pair.

Official pages read for this wave (DocC through `tools/parity/docc.py`):
- https://developer.apple.com/documentation/corehid/hiddevicemanager
- https://developer.apple.com/documentation/corehid/hiddevicemanager/devicematchingcriteria
- https://developer.apple.com/documentation/corehid/hiddevicetransport

Matching rules: a criteria set is an AND of the fields it sets. The criteria list is an OR, because the monitor takes an
array of criteria. This follows the official page for `monitorNotifications(matchingCriteria:)`. The rules below are
BifrOSt choices, not Apple facts:
- `deviceUsages` requires every listed usage to be present on the device. A device has a usage when it is the primary
  usage or in its usage list.
- An empty criteria list matches no device.
- `changes()` compares devices by `unique_id` only. A device whose attributes change while its ID stays is not reported.

## REA evidence

- Core HID landing page: `ev_7f49d05524f631bfd4ccb95852d3b194251a0e1f310dfc0a89f87a411b968402`
- HIDDeviceManager page: `ev_8bce602da9703f94ee3955d7554fa386fa644270d75ddaf326e5550d998aae70`

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| Core HID | Device discovery (HIDDeviceManager, DeviceMatchingCriteria) | partial | springboard/ui/src/hid_match.rs, springboard/sdk/src/hid_devices.rs |
| Core HID | Device classification by capability | partial | springboard/ui/src/hid_match.rs (Usage::role, Transport) |

## Still missing

- The shell does not supply device snapshots to the SDK yet. The evdev scan in `springboard/src/bare/input.rs` must build
  `Device` values and call `DeviceManager::update`.
- `modelNumber` and `localizationCode` from the criteria initializer are not modelled.
- The framework delivers matches as an async stream. The BifrOSt manager is synchronous and expects snapshots.
- Usage pages beyond the listed ones are not named; unknown usages classify as no role.
- Output reports, feature reports and virtual devices are unchanged (`springboard/ui/src/hid_report.rs`, partial).

Not built or run: per the wave rules, no cargo build or tests were run in this round.
