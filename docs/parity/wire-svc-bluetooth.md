# Wire: Bluetooth system service (`bluetooth`)

Branch `wire/svc-bluetooth`. Wires the Bluetooth parity models into the desktop through the named service channel
(`docs/bifrost-ui.md` "System services"). No wire protocol change (`springboard/proto` untouched).

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/iobluetooth.json`) | BifrOSt path |
|---|---|---|
| iobluetooth | Remote device model (name, class, paired, connected) | `springboard/sdk/src/bluetooth_service.rs` (`Bluetooth::devices` is `bluetooth_devices::DeviceList`) |
| iobluetooth | Connect, disconnect and forget a device | `springboard/src/shell/services/bluetooth_service.rs` (`connect`, `disconnect`, `forget`, `pair` methods) |
| iobluetooth | User notifications on device events | `springboard/sdk/src/bluetooth_service.rs` (`apply_event`, `take_events` on `bluetooth_events::DeviceEvents`) |
| iobluetooth | Device inquiry (scan) | `scan` method and the `scanning` flag in `status` (results come back through `status`, not a callback) |

Shell side (`springboard/src/shell/services/bluetooth_service.rs`, service name `bluetooth`):

- `status` returns `{state, scanning, summary, busy, error, devices}`. Each device is `{addr, name, label, kind, paired, connected, audio, rssi}`, paired first, the same order Settings shows.
- `enable(bool)`, `scan`, `pair(addr)`, `connect(addr)`, `disconnect(addr)`, `forget(addr)` make the requests Settings makes. An address must name a device the shell already knows; a radio that is off or missing gives an error reply.
- Events `connected` and `disconnected`, body `{addr, name}`, go to every running app when a device's connection changes. The first tick only records the state, so start-up sends nothing.
- A service cannot return effects, so commands go into a queue (cap 256). `shell/mod.rs` `Shell::tick` drains it once per tick into `Effect::Bluetooth`, which the main loop already sends to the radio thread (`bare/bluetooth.rs`). `bare/bluetooth.rs` itself is unchanged.

SDK side (`springboard/sdk/src/bluetooth_service.rs`, typed client):

- Calls: `status`, `enable`, `scan`, `pair`, `connect`, `disconnect`, `forget`, each returning the request id (`service::call`). The reply comes as `App::service_reply` or `service::take_reply`.
- `Bluetooth::new()` holds the app's copy: `RadioState`, `scanning`, `busy`, `error`, a `DeviceList`, and a `DeviceEvents` registration for every connection event.
- `apply_status(&Json)` replaces the copy from a `status` reply. `apply_event(event, body)` takes a service event and returns true for a Bluetooth connection event. `take_events()` returns the queued `DeviceEvent`s.

## How a user reaches it

- Settings › Bluetooth is unchanged. It already lists paired and nearby devices and offers Connect, Disconnect and Forget.
- Apps reach the feature through the service channel: `service::call(ctx, "bluetooth", "status", &Json::Null)`, or the typed client in `bifrost_app::bluetooth_service`. No app in this change calls it yet, so no app-visible UI is added here.

## How to verify on Midget

Midget is the only test machine (no VM). Use `bifrost pc midget` and the controls in `CLAUDE.md`.

1. Build and publish the release as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget state`, then open Settings › Bluetooth. The device list and the On/Off switch must match the
   state before the update, and toggling the radio must still work.
3. Service check: `bifrost pc control` against an app that calls `bluetooth` `status` (see the `bifrost-app-control`
   skill), or add a call to `bifrost-testapp`. A reply with `"state"`, `"devices"` and the paired devices of Midget
   shows the channel works.
4. Events: connect a paired headset from Settings. The calling app should then receive one `connected` event with the
   device's `addr` and `name`, and one `disconnected` event when it is disconnected.
5. Unit tests (host, run by the lead): `springboard/src/shell/services/bluetooth_service.rs` tests and
   `springboard/sdk/src/bluetooth_service.rs` tests.

## REA evidence

- `ev_13968174b902e40c51404431d77aedd9647507c73e949ff68ef5e3e1bfca2e8e` (IOBluetoothDevice, rendered official page:
  properties, connection methods, the paired and connected checks).
- `ev_e02c7417062086993a909f42ce9dca559e26d7e4edbc4beabf243bcbe6b6f381` (IOBluetoothUserNotification, rendered page:
  `unregister()`).
- Official text (`tools/parity/docc.py`): `iobluetooth/iobluetoothdevice` (an instance represents one remote device)
  and `iobluetooth/iobluetoothusernotification` (a registered notification, `unregister()`).

## Remaining gaps

- No app-visible UI for the feature and no app calls the service yet.
- Pairing has no user-entered PIN or passkey. Only Just Works and the fixed legacy PIN are handled (see the matrix row for pairing).
- Device class is not on the wire, so the SDK model's `class` and `major_class` stay empty; `kind` (BR/EDR or LE) is.
- Events cover connect and disconnect only. Radio state changes and scan results are read through `status`, not pushed.
- The command queue drops requests beyond 256 in one tick (a flood guard, not a limit an app should reach).
- No inquiry delegate or callback API, no L2CAP, RFCOMM, OBEX or hands-free access. These stay partial or OS-only in the matrix.
