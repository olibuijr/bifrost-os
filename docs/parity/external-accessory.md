# External Accessory

Framework: External Accessory, https://developer.apple.com/documentation/externalaccessory (iOS 3.0, macOS 10.13 per the matrix).
Branch: parity/team-k-accessory-access (built on arm). Not yet verified on Midget.

## What was built

- `springboard/sdk/src/external_accessory.rs`: `Accessory::from_core` builds the app view of an accessory from the USB
  accessory manager record (`bifrost_core::accessory_manager::Accessory`). It copies the connection id, the name,
  manufacturer, model and serial strings and the declared protocol strings, and follows the connected state. This
  gives the `EAAccessory` identity fields one implementation shared with the Accessory Access model.
- Test `app_view_copies_identity_and_connected_state_from_the_usb_manager` covers the copy and the disconnect.

## Rows changed

- Accessory object (identity and state): `partial`, now with a path in the SDK.

## Not built, and why

- USB and serial accessories (row `Gap`): this framework does not cover USB or serial devices. Apple's
  External Accessory uses MFi protocols over Lightning, 30-pin or Bluetooth. The USB side is the Accessory Access row
  (see `docs/parity/accessory-access.md`) and serial is not an Apple accessory API. Nothing was added for this row.
- Accessory manager notifications and sessions: no bus source feeds the manager, and the sandbox binds no accessory
  device nodes (`springboard/src/bare/app_policy.rs` binds only `/dev/snd` and `/dev/dri`).
- Protocol byte streams, the Bluetooth picker and the Wi-Fi unconfigured accessory browser are unchanged.

## REA evidence

`ev_8107521ea624a0b2cff20b64e9290446d5423c8f9d9d6999327d89ec754fcab6` (landing page, from the matrix row).
