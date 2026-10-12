# Wave 5: IOBluetooth parity (device inquiry, remote device model, HID devices)

Branch `parity/w5-iobluetooth`, started from `parity/integration` (waves 1 to 4).

## What was built

Three pure std-only SDK models in `springboard/sdk/src/`. Each has its own `#[cfg(test)]` tests with hand-worked
expected values. Each is wired with one `pub mod` line in `springboard/sdk/src/lib.rs`. None opens a radio or a socket,
and none is called by the shell or an app yet.

- `bluetooth_devices.rs` (`bifrost_app::bluetooth_devices`): `RemoteDevice` (address, class of device, name, paired,
  connected, RSSI) and `DeviceList`. It keeps one entry per address, merges inquiry reports without clearing the
  paired or connected state, and orders the list as Settings does: paired first, then by label ignoring case.
  Mirrors the Apple remote device properties `name`, `classOfDevice`, `nameOrAddress`, `deviceClassMajor`.
- `bluetooth_inquiry.rs` (`bifrost_app::bluetooth_inquiry`): `Inquiry` with `start`, `report`, `stop`, `elapse`,
  `found`, `clear_found`. It emits `Started`, `Found(address)` once per device, and `Complete { aborted, found }`. The
  length is clamped to 1 to 61 seconds. Mirrors `IOBluetoothDeviceInquiry` (`start`, `stop`, `foundDevices`,
  `clearFoundDevices`, `inquiryLength`) and its delegate callbacks `deviceInquiryStarted`, `deviceInquiryDeviceFound`,
  `deviceInquiryComplete`.
- `bluetooth_hid.rs` (`bifrost_app::bluetooth_hid`): `is_hid` (peripheral major class), `IgnoreList`, `HidCounts` and
  `counts`. An ignored HID device is left out of the connected and paired counts. The ignore list does not change
  pairing or connection. Devices with no known class are never counted.

Shared dependency: `bifrost_core::bt_address` (`BdAddr`, `ClassOfDevice`, `MajorClass`), already on the branch.

## Official API names

- Classes and methods (DocC, fetched): `IOBluetoothDevice` (name, classOfDevice, nameOrAddress, deviceClassMajor),
  `IOBluetoothDeviceInquiry` (start, stop, foundDevices, clearFoundDevices, inquiryLength),
  `IOBluetoothDeviceInquiryDelegate` (deviceInquiryStarted, deviceInquiryDeviceFound, deviceInquiryComplete).
  Source: https://developer.apple.com/documentation/iobluetooth/iobluetoothdevice,
  https://developer.apple.com/documentation/iobluetooth/iobluetoothdeviceinquiry,
  https://developer.apple.com/documentation/iobluetooth/iobluetoothdeviceinquirydelegate
- HID functions (`IOBluetoothIgnoreHIDDevice`, `IOBluetoothRemoveIgnoredHIDDevice`, the HID count functions): the
  matrix row names them, but the DocC paths tried for them returned no page (HTTP error from the DocC service). The
  names are therefore not verified from the official documentation, and the HID model mirrors the concept only.

## REA evidence

- `ev_1c167973391196f6eac315ed766efbdc0a6d788eee32357582ec7b759c0e7e93`: rendered page
  https://developer.apple.com/documentation/iobluetooth (IOBluetooth landing page, class and protocol list).

## Matrix rows closed (new status: partial)

| Framework | Feature (as written in data/iobluetooth.json) | BifrOSt path |
|---|---|---|
| iobluetooth | Device inquiry (scan) | springboard/sdk/src/bluetooth_inquiry.rs |
| iobluetooth | Remote device model (name, class, paired, connected) | springboard/sdk/src/bluetooth_devices.rs |
| iobluetooth | HID device handling (ignore, counts) | springboard/sdk/src/bluetooth_hid.rs |

All three stay partial. Nothing feeds the models from the shell (`springboard/src/bare/bluetooth.rs` has its own
device table and scan loop), and no app reaches them. The shell's `Device` struct is not replaced.

## Not done in this wave

- Connect, disconnect and forget (os) and pairing (os) are not closed. The model stores the connected and paired flags
  but has no connection or pairing state machine, so closing those rows would overstate what exists.
- The host controller row (os) is not touched, since it needs the radio.

## Checks

- `python3 tools/check-names.py`: exit 0, no hits.
- No cargo or rustc was run, per the task rules. The tests are written but have not been executed.
