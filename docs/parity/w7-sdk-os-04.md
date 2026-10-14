# Parity sdk-os-04: app-facing Wi-Fi, Bluetooth, nearby and favourite models

This round moved eleven `os` rows to `partial`. Each row cites the SDK model that an app reads or plans with. The
headless tests are in `springboard/sdk/tests/parity_sdk-os-04.rs`.

## What mirrors what

| Matrix row | SDK module | Mirrors |
|---|---|---|
| core-wlan, client | `sdk/src/wifi_status.rs`, `sdk/src/wlan_events.rs` | the connection snapshot and its change notifications |
| core-wlan, interfaces | `sdk/src/wifi_status.rs` | the state, address and signal of the one wireless link |
| core-wlan, scan | `sdk/src/wlan.rs` | scan access points merged into networks with channel, band, PHY and security |
| core-wlan, join | `sdk/src/wlan_keychain.rs`, `sdk/src/wifi_secrets.rs` | passphrase rules and per-network secret storage |
| iobluetooth, host controller | `sdk/src/bluetooth_service.rs`, `sdk/src/bluetooth_controller.rs` | radio state and the local name check |
| iobluetooth, pairing | `sdk/src/bluetooth_service.rs`, `sdk/src/bluetooth_picker.rs` | the pairing in progress and the device the user picks |
| iobluetooth, connect and forget | `sdk/src/bluetooth_devices.rs`, `sdk/src/bluetooth_events.rs`, `sdk/src/bluetooth_service.rs` | device order, forgetting, connect and disconnect events |
| cloudkit, local storage | `sdk/src/defaults.rs`, `sdk/src/coding.rs` | the per-app defaults store (local only) |
| multipeer, browsing | `sdk/src/nearby_service.rs`, `sdk/src/nearby_browsing.rs` | service type rules and the browse state |
| multipeer, background | `sdk/src/nearby_browsing.rs` | a browse suspended in the background and resumed in the foreground |
| photokit, favourites | `sdk/src/photo_library.rs`, `ui/src/photo_request.rs` | the favourite flag on an asset record and the favourite change plan |

## What is wired

- The SDK models read the values the shell already publishes (the Wi-Fi status, the Bluetooth status reply, the photo
  asset record). They make no service call of their own.
- `nearby_browsing` and `bluetooth_controller` are new in this round. Neither is called by the shell yet.

## What needs the lab machine

- The Wi-Fi join, select and forget requests, and the Bluetooth connect, disconnect and pairing round trips, run in the
  shell and netd. They are verified only on the lab machine.
- The nearby beacon does not yet take an app's service type or its foreground state.
- The favourite write runs in the shell. The SDK has no change request call for apps.

## Not done in this round

- Cloud user identity (CKUserIdentity) has no app-facing model. The account server is outside this repository.
- Sandbox access to udev data (disk-arbitration) and the GL context requirements (glkit) are system policy. No SDK
  surface was added for them.
- ChaCha20-Poly1305 (apple-cryptokit) needs a Poly1305 implementation. It was not written without a test run.
