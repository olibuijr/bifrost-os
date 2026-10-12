# External Accessory parity (wave 5)

Two rows from the External Accessory matrix (`docs/parity-matrix/data/external-accessory.json`) are now modelled in
pure Rust in `springboard/sdk`: the Bluetooth accessory picker (status `os`) and Wi-Fi setup for unconfigured
accessories (status `gap`). Both are pure models. They read no radio, start no scan, join no network and pair no device.
The shell and the Wi-Fi service do not call them yet, so both rows are `partial`, not `parity`.

## What was built

- `springboard/sdk/src/bluetooth_picker.rs` (`pub mod bluetooth_picker;` in `springboard/sdk/src/lib.rs`).
  `BluetoothPicker::new(name_filter)` keeps a case-insensitive name filter. The shell passes the nearby devices with
  `offer`, which applies the filter, drops repeated addresses (the first name wins) and returns the number shown.
  `select(address)` accepts only a shown device, so a device hidden by the filter is refused and the picker stays open.
  `cancel` dismisses it. After `select` or `cancel` the picker takes no more input, and `outcome` reports
  `Outcome::Selected(Candidate)` or `Outcome::Cancelled`. Pairing stays in the shell's Bluetooth service.
- `springboard/sdk/src/wireless_accessory_browser.rs` (`pub mod wireless_accessory_browser;` in
  `springboard/sdk/src/lib.rs`). `WirelessBrowser::new(may_configure)` takes the app's configuration grant as a plain
  flag. `start_searching(name_filter)` clears the results and begins a search, `stop_searching` ends it and keeps the
  results, and `found(accessory)` is called by the system. A duplicate MAC address is refused, and an accessory is
  refused while the browser is idle or when its name fails the filter. `configure(mac, ssid, passphrase)` checks, in
  this order, the grant, that the accessory was found, the network name (1 to 32 bytes) and the passphrase (8 to 63
  printable ASCII bytes, or 64 hex digits as a raw key). It returns a `ConfigRequest`, whose `Debug` output hides the
  passphrase. Nothing sends the request.

Neither module has a dependency beyond `std` and `bifrost_core::bt_address::{BdAddr, DeviceKind}`, which the
Bluetooth events module already uses.

## Official API names mirrored

- `EAAccessoryManager`: `showBluetoothAccessoryPicker(withNameFilter:completion:)`, with the completion's selected
  accessory or cancellation. Source: https://developer.apple.com/documentation/externalaccessory/eaaccessorymanager
- `EAWiFiUnconfiguredAccessoryBrowser`: `startSearchingForUnconfiguredAccessories(matching:)`,
  `stopSearchingForUnconfiguredAccessories()`, `unconfiguredAccessories` and `configureAccessory(_:withConfigurationUIOn:)`.
  Source: https://developer.apple.com/documentation/externalaccessory/eawifiunconfiguredaccessorybrowser
- `EAWiFiUnconfiguredAccessory`: `name`, `manufacturer`, `model`, `ssid`, `macAddress`. Source:
  https://developer.apple.com/documentation/externalaccessory/eawifiunconfiguredaccessory
- The "may configure" property on the Wi-Fi browser is the Wireless Accessory Configuration entitlement, which the
  framework page names as a Boolean.

Read from the official pages with `python3 -I tools/parity/docc.py`. The Apple predicate for the Wi-Fi search is
modelled as a name substring, because the model has no other fields to match on. The Apple option set for accessory
properties is not modelled. The Apple picker's pairing UI is not modelled, and the Wi-Fi configuration UI hand-off is
replaced by the returned request.

## REA evidence

- `ev_02fb43652a876ac9ffa1093f229f715ccfe0a4e8a4d74ba6e00f7c05b13d5ab6`: rendered page
  `https://developer.apple.com/documentation/externalaccessory` (External Accessory landing page). Its topic list
  confirms the picker's parent topic and the Wi-Fi accessory configuration topics named above.

## Matrix rows closed

| Framework | Feature (as in `external-accessory.json`) | New status | BifrOSt path |
|---|---|---|---|
| External Accessory | Bluetooth picker and pairing UI | partial | `springboard/sdk/src/bluetooth_picker.rs` |
| External Accessory | Wi-Fi accessory configuration (unconfigured accessories) | partial | `springboard/sdk/src/wireless_accessory_browser.rs` |

## Not built and why

- USB and serial accessories (`gap`): needs device-node access the sandbox does not grant, so it is outside pure Rust.
- Accessory input and media devices (`os`): the shell's evdev classification in `springboard/src/bare/input.rs` is tied
  to the device ioctl code. Extracting it would edit the shell's input path, which this wave does not build or test, so
  it is left for a later wave.
- The `ConfigRequest` has no consumer. Connecting it to the Wi-Fi service (netd) needs the SDK client for that service,
  which the Wi-Fi notes list as planned and not built.

## Tests

Each module has a `#[cfg(test)] mod tests` block with real behaviour tests, worked out by hand from the rules above:

- `bluetooth_picker`: 5 tests (unfiltered order, filter match ignoring case, repeated address kept once, select
  finishes with the right device, hidden device refused and picker left open, no input after cancel).
- `wireless_accessory_browser`: 8 tests (idle browser refuses, name filter, duplicate MAC refused, grant checked first,
  unknown accessory, 1 to 32 byte network name, WPA2 passphrase rules including 64 hex and 64 non-hex, passphrase
  hidden in `Debug`).

These were not run, because this wave is not allowed to build. The integration build runs them.
