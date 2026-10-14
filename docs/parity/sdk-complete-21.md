# Parity sdk-complete-21: Bluetooth errors, device list and save options

Three pure models in `springboard/ui/src`, tested headless in `springboard/ui/tests/parity_sdk-complete-21.rs`.

| Module | Mirrors | Wired | Needs the lab machine |
|---|---|---|---|
| `bluetooth_errors` (`MgmtStatus`, `AttError`) | Core Bluetooth error domains and ATT error codes | Not yet; the shell keeps `bare::bluetooth::status_text` | Real ATT codes from a paired device |
| `bluetooth_devices` (`device_list`, `DeviceRow`, `LinkState`) | The device list sections and link state rows of the Settings page | Not yet; `shell/pages.rs` builds its rows itself | Pairing and connecting real devices |
| `save_options` (`SaveOptions`, `ImageFormat`) | The image edit save options | Not yet; `shell/photos/edit.rs` saves a copy directly | The save sheet and the edit panel on the device |

Each module is a value type with no device, file or display access. The tests cover the code mapping, text, retry advice, sorting,
file naming and quality clamping.
