# Parity wave 5: IOBluetooth UI (device chooser, passkey display)

Two rows of the IOBluetooth UI matrix (`docs/parity-matrix/data/iobluetooth-ui.json`) move from gap to partial.
Both are pure Rust models in `bifrost-ui` (`springboard/ui/src`), which the SDK re-exports as `bifrost_app::controls`,
so apps reach them without an SDK change. Neither model is wired to the kernel or the Settings page yet.

## What was built

- `springboard/ui/src/bt_chooser.rs`: the device chooser model.
  - `ChooserDevice`: address, name, paired, audio, advertised 16-bit UUIDs.
  - `DeviceFilter`: allowed UUIDs (add once, add array, clear), audio-only, case-insensitive name query.
    An empty allowed list accepts every device; otherwise a device must advertise one allowed UUID.
  - `sections()`: visible devices split into "My Devices" (paired) and "Other Devices", each sorted by name ignoring case, then address.
  - `Chooser`: single or multiple selection, title, header, description, prompt and cancel labels, `confirm`
    (refused with nothing picked), `cancel`, `results()` (only after confirm), `retain_visible()` (drops picks that disappeared or were filtered out).
- `springboard/ui/src/bt_passkey.rs`: the passkey display model.
  - Six-digit passkeys (0 to 999999), `format_passkey` ("123 456") and `parse_entered` (spaces ignored, exactly six ASCII digits).
  - `PasskeyDisplay`: `set_passkey(value, incoming, ssp)` (refused above 999999, state kept), `text`, `passkey`,
    `is_incoming_request`, `uses_ssp` (numeric comparison), indicator advance (stops at 6), retreat (stops at 0), reset,
    and `matches(typed)` for a passkey typed on this side.
- `springboard/ui/src/lib.rs`: two `pub mod` lines (`bt_chooser`, `bt_passkey`).

Tests: `#[cfg(test)] mod tests` in each file, six behaviour tests in `bt_chooser.rs` and six in `bt_passkey.rs`,
expected values worked out by hand from the rules above. Not run: per the wave rules, no builds or tests were run.

## Official API names mirrored

- IOBluetoothDeviceSelectorController (https://developer.apple.com/documentation/iobluetoothui/iobluetoothdeviceselectorcontroller):
  addAllowedUUID(_:), addAllowedUUIDArray(_:), clearAllowedUUIDs(), getResults(), setTitle(_:), setHeader(_:),
  setDescriptionText(_:), setPrompt(_:), setCancel(_:), setOptions(_:) (single or multiple, as a flag, not a bit set).
  Not mirrored: beginSheetModal, runModal, setSearchAttributes, the device-search inquiry, deviceSelector().
- IOBluetoothPasskeyDisplay (https://developer.apple.com/documentation/iobluetoothui/iobluetoothpasskeydisplay):
  setPasskey(_:for:usingSSP:), passkey, isIncomingRequest, advancePasskeyIndicator(), retreatPasskeyIndicator(),
  resetPasskeyIndicator(). Not mirrored: sharedDisplayView, the view and image properties (no drawing in the model),
  and the passkey notifications flag.

The reference passkey is a string; the model stores the value as an integer and formats it.

## REA evidence

- Device selector page: `ev_69ebec0874775752908e59f4189876cdb8ed1ccdcc6ecc641eb5934959ed4563`
- Passkey display page: `ev_abafd9cbc929af844d8c476be860521d3fbb9e6e9f17b94cb15ac612383c0303`
- Landing page (from the matrix): `ev_d6b887143eeccce4bacc4a630a1c8eef5b2d29145c91c5b40815e6577b0000f5`

## Matrix rows closed

| Framework | Feature (as in data/iobluetooth-ui.json) | New status | BifrOSt path |
|---|---|---|---|
| IOBluetooth UI | Device selector panel (IOBluetoothDeviceSelectorController) | partial | springboard/ui/src/bt_chooser.rs |
| IOBluetooth UI | Passkey and confirmation display (IOBluetoothPasskeyDisplay) | partial | springboard/ui/src/bt_passkey.rs |

## Still missing (for the lead)

- The Settings Bluetooth page does not use the chooser, and pairing still uses NoInputNoOutput (`springboard/src/bare/bluetooth.rs`).
  The kernel's user-confirm request is still answered automatically, so the passkey model has no caller yet.
- No sheet or panel renders either model. The shell must draw it.
- The service browser, object push and the pairing panel's title and prompt options stay as in the matrix.
